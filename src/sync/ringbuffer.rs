use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use portable_atomic::{AtomicUsize, Ordering};

#[repr(C, align(32))]
pub struct RingBuffer<T, const N: usize> {
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    _pad: [u8; 16],
}

unsafe impl<T: Send, const N: usize> Send for RingBuffer<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for RingBuffer<T, N> {}

impl<T, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        assert!(N > 0 && (N & (N - 1)) == 0, "N must be a power of 2");

        Self {
            buffer: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            _pad: [0; 16],
        }
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len() >= N
    }

    #[inline(always)]
    pub fn available_write(&self) -> usize {
        N - self.len()
    }

    #[inline(always)]
    pub fn available_read(&self) -> usize {
        self.len()
    }

    #[inline(always)]
    const fn mask(&self) -> usize {
        N - 1
    }
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    #[inline]
    pub unsafe fn write_slice(&self) -> &mut [T] {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        let available = N - head.wrapping_sub(tail);
        if available == 0 {
            return &mut [];
        }

        let head_idx = head & self.mask();
        let tail_idx = tail & self.mask();

        let contiguous = if head_idx >= tail_idx {
            N - head_idx
        } else {
            tail_idx - head_idx
        }.min(available);

        let ptr = (*self.buffer.get()).as_mut_ptr().add(head_idx) as *mut T;
        core::slice::from_raw_parts_mut(ptr, contiguous)
    }

    #[inline(always)]
    pub unsafe fn commit_write(&self, len: usize) {
        let head = self.head.load(Ordering::Relaxed);
        self.head.store(head.wrapping_add(len), Ordering::Release);
    }

    #[inline]
    pub unsafe fn read_slice(&self) -> &[T] {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);

        let available = head.wrapping_sub(tail);
        if available == 0 {
            return &[];
        }

        let head_idx = head & self.mask();
        let tail_idx = tail & self.mask();

        let contiguous = if head_idx > tail_idx {
            head_idx - tail_idx
        } else {
            N - tail_idx
        }.min(available);

        let ptr = (*self.buffer.get()).as_ptr().add(tail_idx) as *const T;
        core::slice::from_raw_parts(ptr, contiguous)
    }

    #[inline(always)]
    pub unsafe fn commit_read(&self, len: usize) {
        let tail = self.tail.load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(len), Ordering::Release);
    }

    #[inline]
    pub fn try_push(&self, value: T) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        if head.wrapping_sub(tail) >= N {
            return false;
        }

        let idx = head & self.mask();
        unsafe {
            let ptr = (*self.buffer.get()).as_mut_ptr().add(idx);
            (ptr as *mut T).write(value);
        }

        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    #[inline]
    pub fn try_pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);

        if head == tail {
            return None;
        }

        let idx = tail & self.mask();
        let value = unsafe {
            let ptr = (*self.buffer.get()).as_ptr().add(idx);
            (ptr as *const T).read()
        };

        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    #[inline]
    pub fn clear(&self) {
        let head = self.head.load(Ordering::Relaxed);
        self.tail.store(head, Ordering::Release);
    }
}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

pub type ByteRingBuffer<const N: usize> = RingBuffer<u8, N>;

pub type RingBuffer256 = RingBuffer<u8, 256>;

pub type RingBuffer512 = RingBuffer<u8, 512>;

pub type RingBuffer1K = RingBuffer<u8, 1024>;

pub type RingBuffer4K = RingBuffer<u8, 4096>;

impl<const N: usize> RingBuffer<u8, N> {
    pub fn write(&self, data: &[u8]) -> usize {
        let mut written = 0;
        let mut remaining = data;

        while !remaining.is_empty() && !self.is_full() {
            let slice = unsafe { self.write_slice() };
            if slice.is_empty() {
                break;
            }

            let to_write = slice.len().min(remaining.len());
            slice[..to_write].copy_from_slice(&remaining[..to_write]);

            unsafe { self.commit_write(to_write) };

            written += to_write;
            remaining = &remaining[to_write..];
        }

        written
    }

    pub fn read(&self, buffer: &mut [u8]) -> usize {
        let mut read_total = 0;
        let mut remaining = buffer;

        while !remaining.is_empty() && !self.is_empty() {
            let slice = unsafe { self.read_slice() };
            if slice.is_empty() {
                break;
            }

            let to_read = slice.len().min(remaining.len());
            remaining[..to_read].copy_from_slice(&slice[..to_read]);

            unsafe { self.commit_read(to_read) };

            read_total += to_read;
            remaining = &mut remaining[to_read..];
        }

        read_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let buf: RingBuffer<u32, 8> = RingBuffer::new();

        assert!(buf.is_empty());
        assert!(!buf.is_full());
        assert_eq!(buf.capacity(), 8);

        assert!(buf.try_push(1));
        assert!(buf.try_push(2));
        assert!(buf.try_push(3));

        assert_eq!(buf.len(), 3);

        assert_eq!(buf.try_pop(), Some(1));
        assert_eq!(buf.try_pop(), Some(2));
        assert_eq!(buf.len(), 1);

        buf.clear();
        assert!(buf.is_empty());
    }
}
