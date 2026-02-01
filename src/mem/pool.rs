//! 内存池分配器
//!
//! 提供固定大小块的高效内存分配，支持 DRAM 和 PSRAM 后端。
//! 使用无锁位图追踪实现 O(1) 分配和释放。
//!
//! # 特性
//!
//! - 零拷贝: 分配的内存可以直接使用
//! - 无锁: 使用原子操作实现线程安全
//! - 确定性: O(1) 时间复杂度的分配和释放
//! - 灵活后端: 支持 DRAM (低延迟) 和 PSRAM (大容量)
//!
//! # 示例
//!
//! ```rust,ignore
//! use rustrtos::mem::pool::{MemoryPool, Backend};
//!
//! #[derive(Default)]
//! struct SensorData {
//!     timestamp: u64,
//!     value: f32,
//! }
//!
//! // 创建 DRAM 内存池，32 个槽位
//! static POOL: MemoryPool<SensorData, 32, { Backend::Dram as u8 }> = MemoryPool::new();
//!
//! // 分配
//! let mut data = POOL.alloc().unwrap();
//! data.timestamp = 12345;
//! data.value = 3.14;
//!
//! // 自动释放 (Drop)
//! drop(data);
//! ```

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::Ordering;
// Xtensa 不原生支持 AtomicU64，使用 portable_atomic
use portable_atomic::AtomicU64;

/// 内存后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Backend {
    /// DRAM: 内部 RAM，低延迟 (~10ns)
    Dram = 0,
    /// PSRAM (缓存模式): 外部 RAM，中等延迟
    PsramCached = 1,
    /// PSRAM (直接模式): 外部 RAM，适合 DMA
    PsramDirect = 2,
    /// 自动选择: 根据大小和用途决定
    Auto = 3,
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Dram
    }
}

/// 内存池错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    /// 内存池已满
    PoolFull,
    /// 无效的槽位索引
    InvalidSlot,
    /// 双重释放
    DoubleFree,
    /// 未初始化
    NotInitialized,
}

/// 位图追踪器 (支持最多 64 个槽位)
struct Bitmap64 {
    bits: AtomicU64,
}

impl Bitmap64 {
    const fn new() -> Self {
        Self {
            bits: AtomicU64::new(0),
        }
    }
    
    /// 分配一个空闲槽位
    fn alloc(&self) -> Option<usize> {
        loop {
            let current = self.bits.load(Ordering::Acquire);
            
            // 查找第一个 0 位
            let free_bit = (!current).trailing_zeros();
            if free_bit >= 64 {
                return None; // 全满
            }
            
            let new_bits = current | (1u64 << free_bit);
            
            if self.bits
                .compare_exchange_weak(current, new_bits, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(free_bit as usize);
            }
            // CAS 失败，重试
        }
    }
    
    /// 释放槽位
    fn free(&self, index: usize) -> Result<(), PoolError> {
        if index >= 64 {
            return Err(PoolError::InvalidSlot);
        }
        
        loop {
            let current = self.bits.load(Ordering::Acquire);
            let mask = 1u64 << index;
            
            if current & mask == 0 {
                return Err(PoolError::DoubleFree);
            }
            
            let new_bits = current & !mask;
            
            if self.bits
                .compare_exchange_weak(current, new_bits, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
        }
    }
    
    /// 获取已分配数量
    fn count(&self) -> usize {
        self.bits.load(Ordering::Relaxed).count_ones() as usize
    }
    
    /// 检查槽位是否已分配
    fn is_allocated(&self, index: usize) -> bool {
        if index >= 64 {
            return false;
        }
        (self.bits.load(Ordering::Relaxed) & (1u64 << index)) != 0
    }
}

/// 大位图追踪器 (支持更多槽位)
struct BitmapLarge<const WORDS: usize> {
    bits: [AtomicU64; WORDS],
}

impl<const WORDS: usize> BitmapLarge<WORDS> {
    const fn new() -> Self {
        // 使用 const 初始化
        const INIT: AtomicU64 = AtomicU64::new(0);
        Self {
            bits: [INIT; WORDS],
        }
    }
    
    fn alloc(&self) -> Option<usize> {
        for (word_idx, word) in self.bits.iter().enumerate() {
            loop {
                let current = word.load(Ordering::Acquire);
                
                if current == u64::MAX {
                    break; // 这个 word 已满
                }
                
                let free_bit = (!current).trailing_zeros();
                if free_bit >= 64 {
                    break;
                }
                
                let new_bits = current | (1u64 << free_bit);
                
                if word
                    .compare_exchange_weak(current, new_bits, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    return Some(word_idx * 64 + free_bit as usize);
                }
            }
        }
        None
    }
    
    fn free(&self, index: usize) -> Result<(), PoolError> {
        let word_idx = index / 64;
        let bit_idx = index % 64;
        
        if word_idx >= WORDS {
            return Err(PoolError::InvalidSlot);
        }
        
        let word = &self.bits[word_idx];
        
        loop {
            let current = word.load(Ordering::Acquire);
            let mask = 1u64 << bit_idx;
            
            if current & mask == 0 {
                return Err(PoolError::DoubleFree);
            }
            
            let new_bits = current & !mask;
            
            if word
                .compare_exchange_weak(current, new_bits, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
        }
    }
    
    fn count(&self) -> usize {
        self.bits
            .iter()
            .map(|w| w.load(Ordering::Relaxed).count_ones() as usize)
            .sum()
    }
}

/// 内存池
///
/// 固定大小块的内存分配器。
///
/// # 类型参数
///
/// - `T`: 存储的数据类型
/// - `N`: 槽位数量 (最大 256)
/// - `BACKEND`: 后端类型 (Backend 枚举值)
pub struct MemoryPool<T, const N: usize, const BACKEND: u8> {
    // 存储槽位
    slots: UnsafeCell<[MaybeUninit<T>; N]>,
    // 位图追踪 (支持最多 256 个槽位)
    bitmap: BitmapLarge<4>, // 4 * 64 = 256 bits
    // 标记
    _marker: PhantomData<T>,
}

impl<T, const N: usize, const BACKEND: u8> MemoryPool<T, N, BACKEND> {
    /// 创建新的内存池
    pub const fn new() -> Self {
        assert!(N <= 256, "Pool size must be <= 256");
        
        Self {
            slots: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
            bitmap: BitmapLarge::new(),
            _marker: PhantomData,
        }
    }
    
    /// 分配一个槽位
    pub fn alloc(&self) -> Result<PoolBox<'_, T, N, BACKEND>, PoolError> {
        let index = self.bitmap.alloc().ok_or(PoolError::PoolFull)?;
        
        if index >= N {
            // 释放刚分配的槽位
            let _ = self.bitmap.free(index);
            return Err(PoolError::PoolFull);
        }
        
        let slot_ptr = unsafe {
            let slots = &mut *self.slots.get();
            slots[index].as_mut_ptr()
        };
        
        Ok(PoolBox {
            ptr: unsafe { NonNull::new_unchecked(slot_ptr) },
            index,
            pool: self,
        })
    }
    
    /// 分配并初始化
    pub fn alloc_init(&self, value: T) -> Result<PoolBox<'_, T, N, BACKEND>, PoolError> {
        let mut boxed = self.alloc()?;
        unsafe {
            boxed.ptr.as_ptr().write(value);
        }
        Ok(boxed)
    }
    
    /// 获取已分配数量
    pub fn allocated_count(&self) -> usize {
        self.bitmap.count().min(N)
    }
    
    /// 获取空闲数量
    pub fn free_count(&self) -> usize {
        N.saturating_sub(self.allocated_count())
    }
    
    /// 获取总容量
    pub const fn capacity(&self) -> usize {
        N
    }
    
    /// 检查是否已满
    pub fn is_full(&self) -> bool {
        self.allocated_count() >= N
    }
    
    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.allocated_count() == 0
    }
    
    /// 获取后端类型
    pub const fn backend(&self) -> Backend {
        match BACKEND {
            0 => Backend::Dram,
            1 => Backend::PsramCached,
            2 => Backend::PsramDirect,
            _ => Backend::Auto,
        }
    }
    
    /// 释放槽位 (内部使用)
    fn release(&self, index: usize) {
        let _ = self.bitmap.free(index);
    }
}

// Safety: MemoryPool 使用原子操作实现线程安全
unsafe impl<T: Send, const N: usize, const BACKEND: u8> Send for MemoryPool<T, N, BACKEND> {}
unsafe impl<T: Send + Sync, const N: usize, const BACKEND: u8> Sync for MemoryPool<T, N, BACKEND> {}

/// 内存池分配的智能指针
///
/// 类似 Box<T>，但数据存储在内存池中。
/// 当 PoolBox drop 时自动释放槽位。
pub struct PoolBox<'a, T, const N: usize, const BACKEND: u8> {
    ptr: NonNull<T>,
    index: usize,
    pool: &'a MemoryPool<T, N, BACKEND>,
}

impl<'a, T, const N: usize, const BACKEND: u8> PoolBox<'a, T, N, BACKEND> {
    /// 获取槽位索引
    pub fn index(&self) -> usize {
        self.index
    }
    
    /// 获取原始指针
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }
    
    /// 获取可变原始指针
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }
    
    /// 获取后端类型
    pub fn backend(&self) -> Backend {
        self.pool.backend()
    }
}

impl<'a, T, const N: usize, const BACKEND: u8> Deref for PoolBox<'a, T, N, BACKEND> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T, const N: usize, const BACKEND: u8> DerefMut for PoolBox<'a, T, N, BACKEND> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T, const N: usize, const BACKEND: u8> Drop for PoolBox<'a, T, N, BACKEND> {
    fn drop(&mut self) {
        // 调用 T 的析构函数
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
        }
        // 释放槽位
        self.pool.release(self.index);
    }
}

// Safety: PoolBox 的安全性继承自 MemoryPool
unsafe impl<'a, T: Send, const N: usize, const BACKEND: u8> Send for PoolBox<'a, T, N, BACKEND> {}
unsafe impl<'a, T: Sync, const N: usize, const BACKEND: u8> Sync for PoolBox<'a, T, N, BACKEND> {}

/// 内存池统计
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// 总容量
    pub capacity: usize,
    /// 已分配数量
    pub allocated: usize,
    /// 空闲数量
    pub free: usize,
    /// 后端类型
    pub backend: Backend,
}

impl<T, const N: usize, const BACKEND: u8> MemoryPool<T, N, BACKEND> {
    /// 获取统计信息
    pub fn stats(&self) -> PoolStats {
        let allocated = self.allocated_count();
        PoolStats {
            capacity: N,
            allocated,
            free: N.saturating_sub(allocated),
            backend: self.backend(),
        }
    }
}

/// 便捷类型别名
pub type DramPool<T, const N: usize> = MemoryPool<T, N, { Backend::Dram as u8 }>;
pub type PsramPool<T, const N: usize> = MemoryPool<T, N, { Backend::PsramCached as u8 }>;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bitmap64_alloc_free() {
        let bitmap = Bitmap64::new();
        
        // 分配
        let idx0 = bitmap.alloc().unwrap();
        assert_eq!(idx0, 0);
        
        let idx1 = bitmap.alloc().unwrap();
        assert_eq!(idx1, 1);
        
        // 释放
        bitmap.free(0).unwrap();
        
        // 再次分配应该得到 0
        let idx2 = bitmap.alloc().unwrap();
        assert_eq!(idx2, 0);
    }
    
    #[test]
    fn test_backend_default() {
        assert_eq!(Backend::default(), Backend::Dram);
    }
}
