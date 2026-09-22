use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::mem::psram;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaStrategy {
    Auto,
    ForceDram,
    ForcePsramBounce,
}

impl Default for DmaStrategy {
    fn default() -> Self {
        DmaStrategy::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaState {
    Idle,
    DmaReading,
    DmaWriting,
}

pub const DMA_ALIGNMENT: usize = 32;

pub const AUTO_PSRAM_THRESHOLD: usize = 4096;

#[repr(C, align(32))]
pub struct DmaBuffer<const SIZE: usize> {
    data: UnsafeCell<[u8; SIZE]>,
    state: AtomicBool,
    strategy: DmaStrategy,
    bounce_buffer: Option<NonNull<[u8; SIZE]>>,
}

impl<const SIZE: usize> DmaBuffer<SIZE> {
    pub const fn new(strategy: DmaStrategy) -> Self {
        Self {
            data: UnsafeCell::new([0u8; SIZE]),
            state: AtomicBool::new(false),
            strategy,
            bounce_buffer: None,
        }
    }

    pub const fn new_auto() -> Self {
        Self::new(DmaStrategy::Auto)
    }

    pub const fn size(&self) -> usize {
        SIZE
    }

    pub const fn alignment(&self) -> usize {
        DMA_ALIGNMENT
    }

    pub const fn strategy(&self) -> DmaStrategy {
        self.strategy
    }

    pub fn is_dma_active(&self) -> bool {
        self.state.load(Ordering::Acquire)
    }

    pub fn as_ptr(&self) -> *const u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *const u8
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        self.data.get() as *mut u8
    }

    pub fn as_slice(&self) -> &[u8] {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        unsafe { &*self.data.get() }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        unsafe { &mut *self.data.get() }
    }

    pub fn prepare_for_dma_read(&self) {
        self.state.store(true, Ordering::Release);

        unsafe {
            psram::cache::flush(self.data.get() as *const u8, SIZE);
        }
    }

    pub fn complete_dma_read(&self) {
        self.state.store(false, Ordering::Release);
    }

    pub fn prepare_for_dma_write(&self) {
        self.state.store(true, Ordering::Release);

        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }
    }

    pub fn complete_dma_write(&self) {
        unsafe {
            psram::cache::invalidate(self.data.get() as *const u8, SIZE);
        }

        self.state.store(false, Ordering::Release);
    }

    pub fn fill(&mut self, value: u8) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let slice = unsafe { &mut *self.data.get() };
        slice.fill(value);
    }

    pub fn copy_from_slice(&mut self, src: &[u8]) {
        assert!(!self.is_dma_active(), "Cannot access buffer during DMA");
        let len = src.len().min(SIZE);
        let slice = unsafe { &mut *self.data.get() };
        slice[..len].copy_from_slice(&src[..len]);
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

#[repr(C, align(4))]
pub struct DmaDescriptor {
    pub next: u32,
    pub buffer: u32,
    pub size: u16,
    pub length: u16,
    pub flags: u32,
}

impl DmaDescriptor {
    pub const fn new() -> Self {
        Self {
            next: 0,
            buffer: 0,
            size: 0,
            length: 0,
            flags: 0,
        }
    }

    pub fn set_buffer(&mut self, ptr: *const u8, size: usize) {
        self.buffer = ptr as u32;
        self.size = size as u16;
        self.length = size as u16;
    }

    pub fn link_to(&mut self, next: &DmaDescriptor) {
        self.next = next as *const _ as u32;
    }

    pub fn set_eof(&mut self) {
        self.flags |= 1 << 30;
    }

    pub fn set_owner_dma(&mut self) {
        self.flags |= 1 << 31;
    }

    pub fn is_complete(&self) -> bool {
        (self.flags & (1 << 31)) == 0
    }
}

pub struct DmaBufferBuilder<const SIZE: usize> {
    strategy: DmaStrategy,
    prefill: Option<u8>,
}

impl<const SIZE: usize> DmaBufferBuilder<SIZE> {
    pub const fn new() -> Self {
        Self {
            strategy: DmaStrategy::Auto,
            prefill: None,
        }
    }

    pub const fn with_strategy(mut self, strategy: DmaStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    pub const fn with_prefill(mut self, value: u8) -> Self {
        self.prefill = Some(value);
        self
    }

    pub fn build(self) -> DmaBuffer<SIZE> {
        let mut buf = DmaBuffer::new(self.strategy);
        if let Some(value) = self.prefill {
            buf.fill(value);
        }
        buf
    }
}

pub const fn aligned_size(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

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
