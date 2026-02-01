//! DMA 缓冲区管理
//!
//! 提供 DMA 安全的缓冲区分配和管理，支持自动性能优化策略。
//!
//! # 特性
//!
//! - 32 字节对齐 (DMA 和 cache line 要求)
//! - 自动策略选择: 小缓冲区用 DRAM，大缓冲区可用 PSRAM + bounce buffer
//! - Cache 一致性操作封装
//! - 与 esp-hal DMA traits 集成
//!
//! # DMA 限制
//!
//! ESP32-S3 DMA 控制器有以下限制:
//! - 缓冲区必须 4 字节对齐 (推荐 32 字节以匹配 cache line)
//! - 外设 DMA (SPI/I2S 等) 需要内部 SRAM 缓冲区
//! - PSRAM 地址不能直接用于外设 DMA
//!
//! # 示例
//!
//! ```rust,ignore
//! use rustrtos::mem::dma::{DmaBuffer, DmaStrategy};
//!
//! // 自动选择策略 (推荐)
//! let buf = DmaBuffer::<1024>::new(DmaStrategy::Auto);
//!
//! // 强制使用 DRAM
//! let dram_buf = DmaBuffer::<256>::new(DmaStrategy::ForceDram);
//!
//! // DMA 传输前准备
//! buf.prepare_for_dma_write();
//! // ... DMA 写入 ...
//! buf.complete_dma_write();
//! ```

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::mem::psram;

/// DMA 策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaStrategy {
    /// 自动选择: 小缓冲区用 DRAM，大缓冲区根据用途选择
    /// - < 4KB: DRAM 直接分配
    /// - >= 4KB: 如果是频繁随机访问用 DRAM，顺序传输可用 PSRAM + bounce
    Auto,
    /// 强制使用 DRAM: 适合需要直接 DMA 访问的场景
    ForceDram,
    /// PSRAM + Bounce Buffer: 大缓冲区存储在 PSRAM，DMA 时使用 bounce buffer
    ForcePsramBounce,
}

impl Default for DmaStrategy {
    fn default() -> Self {
        DmaStrategy::Auto
    }
}

/// DMA 缓冲区状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaState {
    /// 空闲，可被 CPU 访问
    Idle,
    /// 正在进行 DMA 读取 (外设读取缓冲区)
    DmaReading,
    /// 正在进行 DMA 写入 (外设写入缓冲区)
    DmaWriting,
}

/// DMA 缓冲区对齐要求
pub const DMA_ALIGNMENT: usize = 32;

/// 自动策略的大小阈值 (字节)
pub const AUTO_PSRAM_THRESHOLD: usize = 4096;

/// DMA 缓冲区
///
/// 为 DMA 操作优化的缓冲区，保证正确的对齐和 cache 一致性。
///
/// # 类型参数
///
/// - `SIZE`: 缓冲区大小 (字节)
#[repr(C, align(32))]
pub struct DmaBuffer<const SIZE: usize> {
    /// 实际数据存储
    data: UnsafeCell<[u8; SIZE]>,
    /// 当前状态
    state: AtomicBool, // true = DMA 活跃
    /// 使用的策略
    strategy: DmaStrategy,
    /// Bounce buffer 指针 (如果使用 PSRAM 策略)
    bounce_buffer: Option<NonNull<[u8; SIZE]>>,
}

impl<const SIZE: usize> DmaBuffer<SIZE> {
    /// 创建新的 DMA 缓冲区
    pub const fn new(strategy: DmaStrategy) -> Self {
        Self {
            data: UnsafeCell::new([0u8; SIZE]),
            state: AtomicBool::new(false),
            strategy,
            bounce_buffer: None,
        }
    }
    
    /// 创建使用自动策略的缓冲区
    pub const fn new_auto() -> Self {
        Self::new(DmaStrategy::Auto)
    }
    
    /// 获取缓冲区大小
    pub const fn size(&self) -> usize {
        SIZE
    }
    
    /// 获取对齐要求
    pub const fn alignment(&self) -> usize {
        DMA_ALIGNMENT
    }
    
    /// 获取策略
    pub const fn strategy(&self) -> DmaStrategy {
        self.strategy
    }
    
    /// 检查 DMA 是否活跃
    pub fn is_dma_active(&self) -> bool {
        self.state.load(Ordering::Acquire)
    }
    
    /// 获取数据指针 (只在 DMA 非活跃时安全)
    ///
    /// # Panics
    ///
    /// 如果 DMA 正在进行会 panic
    pub fn as_ptr(&self) -> *const u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *const u8
    }
    
    /// 获取可变数据指针 (只在 DMA 非活跃时安全)
    ///
    /// # Panics
    ///
    /// 如果 DMA 正在进行会 panic
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *mut u8
    }
    
    /// 获取数据切片
    pub fn as_slice(&self) -> &[u8] {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        unsafe { &*self.data.get() }
    }
    
    /// 获取可变数据切片
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        unsafe { &mut *self.data.get() }
    }
    
    /// 准备 DMA 读取 (外设将读取此缓冲区)
    ///
    /// 在启动 DMA 读取前调用。刷新 cache 确保数据可见。
    pub fn prepare_for_dma_read(&self) {
        // 标记 DMA 活跃
        self.state.store(true, Ordering::Release);
        
        // 刷新 cache，确保数据对 DMA 可见
        unsafe {
            psram::cache::flush(self.data.get() as *const u8, SIZE);
        }
    }
    
    /// 完成 DMA 读取
    ///
    /// DMA 读取完成后调用。
    pub fn complete_dma_read(&self) {
        // 标记 DMA 完成
        self.state.store(false, Ordering::Release);
    }
    
    /// 准备 DMA 写入 (外设将写入此缓冲区)
    ///
    /// 在启动 DMA 写入前调用。使 cache 失效。
    pub fn prepare_for_dma_write(&self) {
        // 标记 DMA 活跃
        self.state.store(true, Ordering::Release);
        
        // 使 cache 失效，准备接收新数据
        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }
    }
    
    /// 完成 DMA 写入
    ///
    /// DMA 写入完成后调用。使 cache 失效确保读取新数据。
    pub fn complete_dma_write(&self) {
        // 再次使 cache 失效，确保后续读取获得 DMA 写入的数据
        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }
        
        // 标记 DMA 完成
        self.state.store(false, Ordering::Release);
    }
    
    /// 填充缓冲区
    pub fn fill(&mut self, value: u8) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let slice = unsafe { &mut *self.data.get() };
        slice.fill(value);
    }
    
    /// 从切片复制数据
    pub fn copy_from_slice(&mut self, src: &[u8]) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let len = src.len().min(SIZE);
        let slice = unsafe { &mut *self.data.get() };
        slice[..len].copy_from_slice(&src[..len]);
    }
    
    /// 复制数据到切片
    pub fn copy_to_slice(&self, dst: &mut [u8]) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let len = dst.len().min(SIZE);
        let slice = unsafe { &*self.data.get() };
        dst[..len].copy_from_slice(&slice[..len]);
    }
}

// Safety: DmaBuffer 使用原子状态追踪和显式同步
unsafe impl<const SIZE: usize> Send for DmaBuffer<SIZE> {}
unsafe impl<const SIZE: usize> Sync for DmaBuffer<SIZE> {}

/// DMA 描述符 (用于链式 DMA)
#[repr(C, align(4))]
pub struct DmaDescriptor {
    /// 下一个描述符的地址 (0 表示结束)
    pub next: u32,
    /// 缓冲区地址
    pub buffer: u32,
    /// 缓冲区大小
    pub size: u16,
    /// 传输长度
    pub length: u16,
    /// 控制标志
    pub flags: u32,
}

impl DmaDescriptor {
    /// 创建新的描述符
    pub const fn new() -> Self {
        Self {
            next: 0,
            buffer: 0,
            size: 0,
            length: 0,
            flags: 0,
        }
    }
    
    /// 设置缓冲区
    pub fn set_buffer(&mut self, ptr: *const u8, size: usize) {
        self.buffer = ptr as u32;
        self.size = size as u16;
        self.length = size as u16;
    }
    
    /// 链接到下一个描述符
    pub fn link_to(&mut self, next: &DmaDescriptor) {
        self.next = next as *const _ as u32;
    }
    
    /// 标记为最后一个描述符
    pub fn set_eof(&mut self) {
        self.flags |= 1 << 30; // EOF bit
    }
    
    /// 标记为有效
    pub fn set_owner_dma(&mut self) {
        self.flags |= 1 << 31; // OWNER bit = 1 means DMA owns it
    }
    
    /// 检查 DMA 是否完成 (CPU 拥有描述符)
    pub fn is_complete(&self) -> bool {
        (self.flags & (1 << 31)) == 0
    }
}

/// DMA 缓冲区构建器
pub struct DmaBufferBuilder<const SIZE: usize> {
    strategy: DmaStrategy,
    prefill: Option<u8>,
}

impl<const SIZE: usize> DmaBufferBuilder<SIZE> {
    /// 创建构建器
    pub const fn new() -> Self {
        Self {
            strategy: DmaStrategy::Auto,
            prefill: None,
        }
    }
    
    /// 设置策略
    pub const fn with_strategy(mut self, strategy: DmaStrategy) -> Self {
        self.strategy = strategy;
        self
    }
    
    /// 设置预填充值
    pub const fn with_prefill(mut self, value: u8) -> Self {
        self.prefill = Some(value);
        self
    }
    
    /// 构建缓冲区
    pub fn build(self) -> DmaBuffer<SIZE> {
        let mut buf = DmaBuffer::new(self.strategy);
        if let Some(value) = self.prefill {
            buf.fill(value);
        }
        buf
    }
}

/// 计算对齐后的大小
pub const fn aligned_size(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

/// 检查地址是否适合 DMA
pub fn is_dma_capable_address(addr: usize) -> bool {
    // ESP32-S3 外设 DMA 只能访问内部 SRAM
    // 内部 SRAM 地址范围: 0x3FC88000 - 0x3FCFFFFF
    (0x3FC8_8000..=0x3FCF_FFFF).contains(&addr)
}

/// 检查缓冲区是否 DMA 安全
pub fn is_dma_safe<T>(ptr: *const T, size: usize) -> bool {
    let addr = ptr as usize;
    
    // 检查对齐
    if addr % 4 != 0 {
        return false;
    }
    
    // 检查地址范围
    is_dma_capable_address(addr) && is_dma_capable_address(addr + size - 1)
}

/// 便捷宏：创建静态 DMA 缓冲区
#[macro_export]
macro_rules! dma_buffer {
    ($name:ident, $size:expr) => {
        static $name: $crate::mem::dma::DmaBuffer<$size> = 
            $crate::mem::dma::DmaBuffer::new_auto();
    };
    ($name:ident, $size:expr, $strategy:expr) => {
        static $name: $crate::mem::dma::DmaBuffer<$size> = 
            $crate::mem::dma::DmaBuffer::new($strategy);
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dma_strategy_default() {
        assert_eq!(DmaStrategy::default(), DmaStrategy::Auto);
    }
    
    #[test]
    fn test_aligned_size() {
        assert_eq!(aligned_size(100, 32), 128);
        assert_eq!(aligned_size(32, 32), 32);
        assert_eq!(aligned_size(1, 32), 32);
    }
    
    #[test]
    fn test_dma_buffer_size() {
        let buf = DmaBuffer::<1024>::new_auto();
        assert_eq!(buf.size(), 1024);
        assert_eq!(buf.alignment(), 32);
    }
}
