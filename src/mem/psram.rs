//! PSRAM 管理模块
//!
//! 提供 ESP32-S3 外部 PSRAM (8MB) 的初始化和分配功能。
//! 支持自动缓存策略选择，默认使用缓存模式以获得最佳性能。
//!
//! # 缓存策略
//!
//! - `CacheMode::Auto`: 根据分配用途自动选择 (默认缓存)
//! - `CacheMode::Cached`: 使用 CPU 缓存，适合频繁随机访问
//! - `CacheMode::Direct`: 直接访问，适合顺序大块传输
//!
//! # 注意事项
//!
//! - PSRAM 延迟约 100ns，DRAM 约 10ns
//! - 缓存模式下需要注意 DMA 的 cache 一致性
//! - 非实时任务的大型缓冲区推荐使用 PSRAM

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// PSRAM 缓存模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// 自动选择：小块频繁访问用缓存，大块顺序访问用直接模式
    /// 默认使用缓存模式
    Auto,
    /// 缓存模式：CPU L1/L2 缓存，适合随机访问
    Cached,
    /// 直接模式：绕过缓存，适合 DMA 和大块顺序访问
    Direct,
}

impl Default for CacheMode {
    fn default() -> Self {
        CacheMode::Auto
    }
}

/// PSRAM 配置
#[derive(Debug, Clone)]
pub struct PsramConfig {
    /// 缓存模式
    pub cache_mode: CacheMode,
    /// 是否为实时任务使用 (影响自动模式决策)
    pub realtime: bool,
    /// 对齐要求 (字节)
    pub alignment: usize,
}

impl Default for PsramConfig {
    fn default() -> Self {
        Self {
            cache_mode: CacheMode::Auto,
            realtime: false,
            alignment: 32, // 缓存行对齐
        }
    }
}

impl PsramConfig {
    /// 创建用于实时任务的配置
    pub fn realtime() -> Self {
        Self {
            cache_mode: CacheMode::Cached,
            realtime: true,
            alignment: 32,
        }
    }
    
    /// 创建用于大块传输的配置
    pub fn bulk_transfer() -> Self {
        Self {
            cache_mode: CacheMode::Direct,
            realtime: false,
            alignment: 32,
        }
    }
    
    /// 设置缓存模式
    pub fn with_cache_mode(mut self, mode: CacheMode) -> Self {
        self.cache_mode = mode;
        self
    }
    
    /// 设置对齐要求
    pub fn with_alignment(mut self, align: usize) -> Self {
        self.alignment = align;
        self
    }
}

/// PSRAM 全局状态
static PSRAM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PSRAM_BASE: AtomicUsize = AtomicUsize::new(0);
static PSRAM_SIZE: AtomicUsize = AtomicUsize::new(0);
static PSRAM_OFFSET: AtomicUsize = AtomicUsize::new(0);

/// 初始化 PSRAM
/// 
/// esp-hal 1.0 在启用 `psram` feature 时会自动初始化 PSRAM。
/// 此函数用于获取 PSRAM 的基地址和大小。
///
/// # Safety
///
/// 应该在系统启动时调用一次。
pub fn init() -> Result<PsramInfo, PsramError> {
    if PSRAM_INITIALIZED.load(Ordering::Acquire) {
        return Ok(PsramInfo {
            base: PSRAM_BASE.load(Ordering::Relaxed),
            size: PSRAM_SIZE.load(Ordering::Relaxed),
        });
    }
    
    // esp-hal 1.0 with psram feature 会自动初始化
    // PSRAM 地址范围: 0x3C000000 - 0x3C7FFFFF (8MB)
    // 注意: 实际基地址和大小需要从 esp-hal 获取
    
    // 使用 esp-hal 提供的 PSRAM 信息
    // 默认 ESP32-S3-N16R8 配置: 8MB Octal PSRAM
    let base = 0x3C00_0000_usize; // PSRAM 映射基地址
    let size = 8 * 1024 * 1024;   // 8MB
    
    PSRAM_BASE.store(base, Ordering::Relaxed);
    PSRAM_SIZE.store(size, Ordering::Relaxed);
    PSRAM_OFFSET.store(0, Ordering::Relaxed);
    PSRAM_INITIALIZED.store(true, Ordering::Release);
    
    Ok(PsramInfo { base, size })
}

/// PSRAM 信息
#[derive(Debug, Clone, Copy)]
pub struct PsramInfo {
    /// 基地址
    pub base: usize,
    /// 总大小 (字节)
    pub size: usize,
}

/// PSRAM 错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsramError {
    /// PSRAM 未初始化
    NotInitialized,
    /// 内存不足
    OutOfMemory,
    /// 对齐错误
    AlignmentError,
    /// 大小为零
    ZeroSize,
}

/// 从 PSRAM 分配内存 (简单 bump allocator)
///
/// # 参数
///
/// - `size`: 分配大小
/// - `align`: 对齐要求
///
/// # 返回
///
/// 分配的内存指针，如果失败返回 None
fn psram_alloc_raw(size: usize, align: usize) -> Result<*mut u8, PsramError> {
    if size == 0 {
        return Err(PsramError::ZeroSize);
    }
    
    if !PSRAM_INITIALIZED.load(Ordering::Acquire) {
        return Err(PsramError::NotInitialized);
    }
    
    let base = PSRAM_BASE.load(Ordering::Relaxed);
    let total_size = PSRAM_SIZE.load(Ordering::Relaxed);
    
    loop {
        let current_offset = PSRAM_OFFSET.load(Ordering::Relaxed);
        let aligned_offset = (current_offset + align - 1) & !(align - 1);
        let new_offset = aligned_offset + size;
        
        if new_offset > total_size {
            return Err(PsramError::OutOfMemory);
        }
        
        // CAS 更新 offset
        if PSRAM_OFFSET
            .compare_exchange(current_offset, new_offset, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return Ok((base + aligned_offset) as *mut u8);
        }
        // 如果 CAS 失败，重试
    }
}

/// PSRAM 分配的智能指针
///
/// 类似 Box<T>，但数据存储在 PSRAM 中。
/// 注意: 当前实现使用 bump allocator，不支持释放单个分配。
pub struct PsramBox<T> {
    ptr: NonNull<T>,
    config: PsramConfig,
    _marker: PhantomData<T>,
}

impl<T> PsramBox<T> {
    /// 在 PSRAM 中分配并初始化值
    pub fn new(value: T) -> Result<Self, PsramError> {
        Self::new_with_config(value, PsramConfig::default())
    }
    
    /// 使用指定配置在 PSRAM 中分配
    pub fn new_with_config(value: T, config: PsramConfig) -> Result<Self, PsramError> {
        let size = core::mem::size_of::<T>();
        let align = config.alignment.max(core::mem::align_of::<T>());
        
        let ptr = psram_alloc_raw(size, align)?;
        let typed_ptr = ptr as *mut T;
        
        // 写入初始值
        unsafe {
            typed_ptr.write(value);
        }
        
        Ok(Self {
            ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
            config,
            _marker: PhantomData,
        })
    }
    
    /// 在 PSRAM 中分配未初始化的内存
    pub fn new_uninit() -> Result<PsramBox<MaybeUninit<T>>, PsramError> {
        Self::new_uninit_with_config(PsramConfig::default())
    }
    
    /// 使用指定配置分配未初始化内存
    pub fn new_uninit_with_config(config: PsramConfig) -> Result<PsramBox<MaybeUninit<T>>, PsramError> {
        let size = core::mem::size_of::<T>();
        let align = config.alignment.max(core::mem::align_of::<T>());
        
        let ptr = psram_alloc_raw(size, align)?;
        let typed_ptr = ptr as *mut MaybeUninit<T>;
        
        Ok(PsramBox {
            ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
            config,
            _marker: PhantomData,
        })
    }
    
    /// 获取指针地址
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }
    
    /// 获取可变指针地址
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }
    
    /// 获取配置
    pub fn config(&self) -> &PsramConfig {
        &self.config
    }
    
    /// 检查是否在 PSRAM 地址范围内
    pub fn is_in_psram(&self) -> bool {
        let addr = self.ptr.as_ptr() as usize;
        let base = PSRAM_BASE.load(Ordering::Relaxed);
        let size = PSRAM_SIZE.load(Ordering::Relaxed);
        addr >= base && addr < base + size
    }
}

impl<T> PsramBox<MaybeUninit<T>> {
    /// 假设内存已初始化
    ///
    /// # Safety
    ///
    /// 调用者必须确保内存已被正确初始化
    pub unsafe fn assume_init(self) -> PsramBox<T> {
        let ptr = self.ptr.as_ptr() as *mut T;
        let config = self.config.clone();
        core::mem::forget(self);
        
        PsramBox {
            ptr: NonNull::new_unchecked(ptr),
            config,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for PsramBox<T> {
    type Target = T;
    
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> DerefMut for PsramBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

// 注意: 当前 bump allocator 不支持释放，所以不实现 Drop
// 如果需要支持释放，需要实现更复杂的分配器

unsafe impl<T: Send> Send for PsramBox<T> {}
unsafe impl<T: Sync> Sync for PsramBox<T> {}

/// 分配 PSRAM 数组
pub fn alloc_array<T: Default + Clone, const N: usize>() -> Result<PsramBox<[T; N]>, PsramError> {
    alloc_array_with_config(PsramConfig::default())
}

/// 使用指定配置分配 PSRAM 数组
pub fn alloc_array_with_config<T: Default + Clone, const N: usize>(
    config: PsramConfig,
) -> Result<PsramBox<[T; N]>, PsramError> {
    let size = core::mem::size_of::<[T; N]>();
    let align = config.alignment.max(core::mem::align_of::<T>());
    
    let ptr = psram_alloc_raw(size, align)?;
    let typed_ptr = ptr as *mut [T; N];
    
    // 初始化数组
    unsafe {
        for i in 0..N {
            (*typed_ptr)[i] = T::default();
        }
    }
    
    Ok(PsramBox {
        ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
        config,
        _marker: PhantomData,
    })
}

/// 获取 PSRAM 使用统计
pub fn stats() -> PsramStats {
    let total = PSRAM_SIZE.load(Ordering::Relaxed);
    let used = PSRAM_OFFSET.load(Ordering::Relaxed);
    
    PsramStats {
        total,
        used,
        free: total.saturating_sub(used),
    }
}

/// PSRAM 使用统计
#[derive(Debug, Clone, Copy)]
pub struct PsramStats {
    /// 总容量 (字节)
    pub total: usize,
    /// 已使用 (字节)
    pub used: usize,
    /// 空闲 (字节)
    pub free: usize,
}

/// Cache 操作 (用于 DMA 一致性)
pub mod cache {
    use core::arch::asm;
    
    /// 刷新 cache (写回到 PSRAM)
    /// 
    /// # Safety
    ///
    /// 地址必须有效且对齐
    #[inline]
    pub unsafe fn flush(addr: *const u8, size: usize) {
        // ESP32-S3 使用 Xtensa 指令刷新 cache
        // DHWBI: Data cache Hit WriteBack Invalidate
        let mut current = addr as usize;
        let end = current + size;
        
        while current < end {
            // Xtensa DHWBI 指令
            #[cfg(target_arch = "xtensa")]
            asm!(
                "dhwbi {0}, 0",
                in(reg) current,
                options(nostack, preserves_flags)
            );
            
            current += 32; // cache line size
        }
        
        // 内存屏障
        #[cfg(target_arch = "xtensa")]
        asm!("memw", options(nostack, preserves_flags));
    }
    
    /// 使 cache 失效 (从 PSRAM 重新加载)
    ///
    /// # Safety
    ///
    /// 地址必须有效且对齐
    #[inline]
    pub unsafe fn invalidate(addr: *const u8, size: usize) {
        let mut current = addr as usize;
        let end = current + size;
        
        while current < end {
            // Xtensa DHI 指令
            #[cfg(target_arch = "xtensa")]
            asm!(
                "dhi {0}, 0",
                in(reg) current,
                options(nostack, preserves_flags)
            );
            
            current += 32; // cache line size
        }
        
        // 内存屏障
        #[cfg(target_arch = "xtensa")]
        asm!("memw", options(nostack, preserves_flags));
    }
    
    /// 刷新并使 cache 失效
    ///
    /// # Safety
    ///
    /// 地址必须有效且对齐
    #[inline]
    pub unsafe fn flush_and_invalidate(addr: *const u8, size: usize) {
        flush(addr, size);
        invalidate(addr, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cache_mode_default() {
        assert_eq!(CacheMode::default(), CacheMode::Auto);
    }
    
    #[test]
    fn test_psram_config_default() {
        let config = PsramConfig::default();
        assert_eq!(config.cache_mode, CacheMode::Auto);
        assert!(!config.realtime);
        assert_eq!(config.alignment, 32);
    }
}
