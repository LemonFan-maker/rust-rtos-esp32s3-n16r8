use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::mem::psram;

pub const DMA_ALIGNMENT: usize = 32;

/// DMA安全缓冲区: 32字节对齐的静态存储, 配合GDMA传输使用。
///
/// ESP32-S3的数据缓存对CPU与GDMA不透明: GDMA直接读写物理内存, 不经过CPU数据缓存。
/// 因此CPU写入后必须由`prepare_tx`回写缓存, GDMA写入后必须由`finish_rx`作废缓存,
/// 否则双方可能读写到过期的缓存行。
///
/// 传输期间`state`标志置位, 常规访问器(`as_slice`等)会断言失败,
/// 防止CPU与DMA同时触碰同一缓冲区。
#[repr(C, align(32))]
pub struct DmaBuffer<const SIZE: usize> {
    data: UnsafeCell<[u8; SIZE]>,
    state: AtomicBool,
}

impl<const SIZE: usize> DmaBuffer<SIZE> {
    /// 编译期约束: 缓冲区大小必须是32(缓存行)的整数倍, 否则作废/回写会波及相邻数据。
    const SIZE_IS_CACHE_MULTIPLE: () =
        assert!(SIZE % DMA_ALIGNMENT == 0, "DmaBuffer size must be a multiple of 32");

    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new([0u8; SIZE]),
            state: AtomicBool::new(false),
        }
    }

    pub const fn size(&self) -> usize {
        SIZE
    }

    pub const fn alignment(&self) -> usize {
        DMA_ALIGNMENT
    }

    pub fn is_dma_active(&self) -> bool {
        self.state.load(Ordering::Acquire)
    }

    /// 准备作为DMA发送源: 回写数据缓存并标记忙, 返回只读切片。
    /// 传输结束后调用`finish_tx`。
    pub fn prepare_tx(&self) -> &[u8] {
        assert!(!self.is_dma_active(), "Buffer already engaged in DMA");
        self.state.store(true, Ordering::Release);
        unsafe {
            psram::cache::flush(self.data.get() as *const u8, SIZE);
        }
        unsafe { &*self.data.get() }
    }

    /// 结束发送: 清除忙标志。
    pub fn finish_tx(&self) {
        self.state.store(false, Ordering::Release);
    }

    /// 准备作为DMA接收目标: 作废数据缓存并标记忙, 返回物理内存写指针。
    /// 传输结束后调用`finish_rx`。忙标志保证同一时刻至多一个传输/借用存在。
    pub fn prepare_rx(&self) -> *mut u8 {
        assert!(!self.is_dma_active(), "Buffer already engaged in DMA");
        self.state.store(true, Ordering::Release);
        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }
        self.data.get() as *mut u8
    }

    /// 结束接收: 再次作废缓存(丢弃传输期间可能的投机预取)并清除忙标志。
    pub fn finish_rx(&self) {
        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }
        self.state.store(false, Ordering::Release);
    }

    pub fn as_ptr(&self) -> *const u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *const u8
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *mut u8
    }

    pub fn as_slice(&self) -> &[u8] {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        unsafe { &*self.data.get() }
    }

    /// 在独占借用下执行CPU写操作(闭包内可安全突变缓冲区)。
    pub fn with_mut(&self, f: impl FnOnce(&mut [u8])) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.state.store(true, Ordering::Release);
        f(unsafe { &mut *self.data.get() });
        self.state.store(false, Ordering::Release);
    }

    pub fn fill(&self, value: u8) {
        self.with_mut(|s| s.fill(value));
    }

    pub fn copy_from_slice(&self, src: &[u8]) {
        let len = src.len().min(SIZE);
        self.with_mut(|s| s[..len].copy_from_slice(&src[..len]));
    }

    pub fn copy_to_slice(&self, dst: &mut [u8]) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let len = dst.len().min(SIZE);
        let slice = unsafe { &*self.data.get() };
        dst[..len].copy_from_slice(&slice[..len]);
    }
}

unsafe impl<const SIZE: usize> Send for DmaBuffer<SIZE> {}
unsafe impl<const SIZE: usize> Sync for DmaBuffer<SIZE> {}

pub const fn aligned_size(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

/// 地址是否位于ESP32-S3内部DRAM数据总线窗口(GDMA可达)。
pub fn is_dma_capable_address(addr: usize) -> bool {
    (0x3FC8_8000..=0x3FCF_FFFF).contains(&addr)
}

pub fn is_dma_safe<T>(ptr: *const T, size: usize) -> bool {
    let addr = ptr as usize;

    if addr % 4 != 0 {
        return false;
    }

    is_dma_capable_address(addr) && is_dma_capable_address(addr + size - 1)
}

#[macro_export]
macro_rules! dma_buffer {
    ($name:ident, $size:expr) => {
        static $name: $crate::mem::dma::DmaBuffer<$size> =
            $crate::mem::dma::DmaBuffer::new();
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aligned_size() {
        assert_eq!(aligned_size(100, 32), 128);
        assert_eq!(aligned_size(32, 32), 32);
        assert_eq!(aligned_size(1, 32), 32);
    }

    #[test]
    fn test_dma_buffer_size() {
        let buf = DmaBuffer::<1024>::new();
        assert_eq!(buf.size(), 1024);
        assert_eq!(buf.alignment(), 32);
    }

    #[test]
    fn test_buffer_alignment() {
        let buf = DmaBuffer::<64>::new();
        assert_eq!(buf.as_ptr() as usize % 32, 0);
    }
}
