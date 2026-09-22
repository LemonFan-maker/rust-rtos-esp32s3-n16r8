use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::Ordering;
use portable_atomic::AtomicU64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Backend {
    Dram = 0,
    PsramCached = 1,
    PsramDirect = 2,
    Auto = 3,
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Dram
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    PoolFull,
    InvalidSlot,
    DoubleFree,
    NotInitialized,
}

struct Bitmap64 {
    bits: AtomicU64,
}

impl Bitmap64 {
    const fn new() -> Self {
        Self {
            bits: AtomicU64::new(0),
        }
    }

    fn alloc(&self) -> Option<usize> {
        loop {
            let current = self.bits.load(Ordering::Acquire);

            let free_bit = (!current).trailing_zeros();
            if free_bit >= 64 {
                return None;
            }

            let new_bits = current | (1u64 << free_bit);

            if self.bits
                .compare_exchange_weak(current, new_bits, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(free_bit as usize);
            }
        }
    }

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

    fn count(&self) -> usize {
        self.bits.load(Ordering::Relaxed).count_ones() as usize
    }

    fn is_allocated(&self, index: usize) -> bool {
        if index >= 64 {
            return false;
        }
        (self.bits.load(Ordering::Relaxed) & (1u64 << index)) != 0
    }
}

struct BitmapLarge<const WORDS: usize> {
    bits: [AtomicU64; WORDS],
}

impl<const WORDS: usize> BitmapLarge<WORDS> {
    const fn new() -> Self {
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
                    break;
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

pub struct MemoryPool<T, const N: usize, const BACKEND: u8> {
    slots: UnsafeCell<[MaybeUninit<T>; N]>,
    bitmap: BitmapLarge<4>,
    _marker: PhantomData<T>,
}

impl<T, const N: usize, const BACKEND: u8> MemoryPool<T, N, BACKEND> {
    pub const fn new() -> Self {
        assert!(N <= 256, "Pool size must be <= 256");

        Self {
            slots: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
            bitmap: BitmapLarge::new(),
            _marker: PhantomData,
        }
    }

    pub fn alloc(&self) -> Result<PoolBox<'_, T, N, BACKEND>, PoolError> {
        let index = self.bitmap.alloc().ok_or(PoolError::PoolFull)?;

        if index >= N {
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

    pub fn alloc_init(&self, value: T) -> Result<PoolBox<'_, T, N, BACKEND>, PoolError> {
        let boxed = self.alloc()?;
        unsafe {
            boxed.ptr.as_ptr().write(value);
        }
        Ok(boxed)
    }

    pub fn allocated_count(&self) -> usize {
        self.bitmap.count().min(N)
    }

    pub fn free_count(&self) -> usize {
        N.saturating_sub(self.allocated_count())
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn is_full(&self) -> bool {
        self.allocated_count() >= N
    }

    pub fn is_empty(&self) -> bool {
        self.allocated_count() == 0
    }

    pub const fn backend(&self) -> Backend {
        match BACKEND {
            0 => Backend::Dram,
            1 => Backend::PsramCached,
            2 => Backend::PsramDirect,
            _ => Backend::Auto,
        }
    }

    fn release(&self, index: usize) {
        let _ = self.bitmap.free(index);
    }
}

unsafe impl<T: Send, const N: usize, const BACKEND: u8> Send for MemoryPool<T, N, BACKEND> {}
unsafe impl<T: Send + Sync, const N: usize, const BACKEND: u8> Sync for MemoryPool<T, N, BACKEND> {}

pub struct PoolBox<'a, T, const N: usize, const BACKEND: u8> {
    ptr: NonNull<T>,
    index: usize,
    pool: &'a MemoryPool<T, N, BACKEND>,
}

impl<'a, T, const N: usize, const BACKEND: u8> PoolBox<'a, T, N, BACKEND> {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }

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
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
        }
        self.pool.release(self.index);
    }
}

unsafe impl<'a, T: Send, const N: usize, const BACKEND: u8> Send for PoolBox<'a, T, N, BACKEND> {}
unsafe impl<'a, T: Sync, const N: usize, const BACKEND: u8> Sync for PoolBox<'a, T, N, BACKEND> {}

#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub capacity: usize,
    pub allocated: usize,
    pub free: usize,
    pub backend: Backend,
}

impl<T, const N: usize, const BACKEND: u8> MemoryPool<T, N, BACKEND> {
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

pub type DramPool<T, const N: usize> = MemoryPool<T, N, { Backend::Dram as u8 }>;
pub type PsramPool<T, const N: usize> = MemoryPool<T, N, { Backend::PsramCached as u8 }>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitmap64_alloc_free() {
        let bitmap = Bitmap64::new();

        let idx0 = bitmap.alloc().unwrap();
        assert_eq!(idx0, 0);

        let idx1 = bitmap.alloc().unwrap();
        assert_eq!(idx1, 1);

        bitmap.free(0).unwrap();

        let idx2 = bitmap.alloc().unwrap();
        assert_eq!(idx2, 0);
    }

    #[test]
    fn test_backend_default() {
        assert_eq!(Backend::default(), Backend::Dram);
    }
}
