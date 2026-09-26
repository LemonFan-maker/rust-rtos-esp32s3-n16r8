use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use portable_atomic::{AtomicBool, AtomicUsize, Ordering};

#[repr(C, align(32))]
pub struct RingBuffer<T, const N: usize> {
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    writer_claim: AtomicBool,
    reader_claim: AtomicBool,
    _pad: [u8; 14],
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
            writer_claim: AtomicBool::new(false),
            reader_claim: AtomicBool::new(false),
            _pad: [0; 14],
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
    /// 声明独占写许可。同一缓冲区同时只允许一个活跃写者:
    /// 上一个`RingWriter`尚未Drop时返回`None`。
    pub fn begin_write(&self) -> Option<RingWriter<'_, T, N>> {
        self.writer_claim
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| RingWriter {
                rb: self,
                base: self.head.load(Ordering::Relaxed),
                written: 0,
            })
    }

    /// 声明独占读许可。同一缓冲区同时只允许一个活跃读者:
    /// 上一个`RingReader`尚未Drop时返回`None`。
    pub fn begin_read(&self) -> Option<RingReader<'_, T, N>> {
        self.reader_claim
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| RingReader {
                rb: self,
                base: self.tail.load(Ordering::Relaxed),
                consumed: 0,
            })
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

/// 写侧RAII守卫: 通过`DerefMut`直接写入环形缓冲区内部存储,
/// `commit(n)`标记已写入元素, Drop时提交写指针并释放写许可。
/// 活跃守卫未Drop前`begin_write()`返回`None`, 从类型层面阻止双写者别名。
///
/// 单写者约定: 持有守卫期间不得再从其他执行流对本缓冲区调用
/// `try_push`或`clear`, 否则写指针更新会互相覆盖。
pub struct RingWriter<'rb, T: Copy, const N: usize> {
    rb: &'rb RingBuffer<T, N>,
    base: usize,
    written: usize,
}

impl<'rb, T: Copy, const N: usize> RingWriter<'rb, T, N> {
    /// 当前可连续写入的元素数(到物理末尾或缓冲区满为止)。
    #[inline]
    pub fn capacity(&self) -> usize {
        let tail = self.rb.tail.load(Ordering::Acquire);
        let pos = self.pos();
        let available = N - pos.wrapping_sub(tail);
        let contiguous = N - (pos & self.rb.mask());
        available.min(contiguous)
    }

    /// 已写入但尚未提交的元素数。
    #[inline]
    pub fn len(&self) -> usize {
        self.written
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }

    /// 标记已通过切片接口初始化了前`n`个可写元素(不超过当前可写数)。
    #[inline]
    pub fn commit(&mut self, n: usize) {
        self.written += n.min(self.capacity());
    }

    /// 从`src`拷贝尽可能多的元素进缓冲区, 返回实际写入数。
    #[inline]
    pub fn extend_from_slice(&mut self, src: &[T]) -> usize {
        let n = src.len().min(self.capacity());
        if n > 0 {
            self.as_mut()[..n].copy_from_slice(&src[..n]);
            self.written += n;
        }
        n
    }

    #[inline(always)]
    fn pos(&self) -> usize {
        self.base.wrapping_add(self.written)
    }
}

impl<T: Copy, const N: usize> Deref for RingWriter<'_, T, N> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        let pos = self.pos();
        let len = self.capacity();
        unsafe {
            let ptr = (*self.rb.buffer.get()).as_ptr().add(pos & self.rb.mask()) as *const T;
            core::slice::from_raw_parts(ptr, len)
        }
    }
}

impl<T: Copy, const N: usize> DerefMut for RingWriter<'_, T, N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        let pos = self.pos();
        let len = self.capacity();
        unsafe {
            let ptr = (*self.rb.buffer.get())
                .as_mut_ptr()
                .add(pos & self.rb.mask()) as *mut T;
            core::slice::from_raw_parts_mut(ptr, len)
        }
    }
}

impl<T: Copy, const N: usize> Drop for RingWriter<'_, T, N> {
    #[inline]
    fn drop(&mut self) {
        self.rb.head.store(self.pos(), Ordering::Release);
        self.rb.writer_claim.store(false, Ordering::Release);
    }
}

/// 读侧RAII守卫: 通过`Deref`零拷贝读取待消费数据,
/// `consume(n)`标记消费, Drop时提交读指针并释放读许可。
///
/// 单读者约定: 持有守卫期间不得再从其他执行流对本缓冲区调用
/// `try_pop`或`clear`, 否则读指针更新会互相覆盖。
pub struct RingReader<'rb, T: Copy, const N: usize> {
    rb: &'rb RingBuffer<T, N>,
    base: usize,
    consumed: usize,
}

impl<'rb, T: Copy, const N: usize> RingReader<'rb, T, N> {
    /// 当前可连续读取的元素数。
    #[inline]
    pub fn len(&self) -> usize {
        let head = self.rb.head.load(Ordering::Acquire);
        let pos = self.pos();
        let available = head.wrapping_sub(pos);
        if available == 0 {
            return 0;
        }
        let head_idx = head & self.rb.mask();
        let pos_idx = pos & self.rb.mask();
        let contiguous = if head_idx > pos_idx {
            head_idx - pos_idx
        } else {
            N - pos_idx
        };
        available.min(contiguous)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 标记消费前`n`个元素; 超过当前可读数的部分被忽略,
    /// 保证读指针永不越过写指针。
    #[inline]
    pub fn consume(&mut self, n: usize) {
        let head = self.rb.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(self.pos());
        self.consumed += n.min(available);
    }

    /// 已消费但尚未提交的元素数。
    #[inline]
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    #[inline(always)]
    fn pos(&self) -> usize {
        self.base.wrapping_add(self.consumed)
    }
}

impl<T: Copy, const N: usize> Deref for RingReader<'_, T, N> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        let pos = self.pos();
        let len = self.len();
        unsafe {
            let ptr = (*self.rb.buffer.get()).as_ptr().add(pos & self.rb.mask()) as *const T;
            core::slice::from_raw_parts(ptr, len)
        }
    }
}

impl<T: Copy, const N: usize> Drop for RingReader<'_, T, N> {
    #[inline]
    fn drop(&mut self) {
        self.rb.tail.store(self.pos(), Ordering::Release);
        self.rb.reader_claim.store(false, Ordering::Release);
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
    /// 批量写入(单写者语义): 循环申请写许可直到`data`写完、
    /// 缓冲区满或写许可被占用。返回实际写入字节数。
    pub fn write(&self, data: &[u8]) -> usize {
        let mut written = 0;
        while written < data.len() {
            let Some(mut writer) = self.begin_write() else {
                break;
            };
            if writer.capacity() == 0 {
                break;
            }
            let n = writer.extend_from_slice(&data[written..]);
            written += n;
        }
        written
    }

    /// 批量读出(单读者语义): 循环申请读许可直到`buffer`填满、
    /// 缓冲区空或读许可被占用。返回实际读出字节数。
    pub fn read(&self, buffer: &mut [u8]) -> usize {
        let mut read_total = 0;
        while read_total < buffer.len() {
            let Some(mut reader) = self.begin_read() else {
                break;
            };
            let n = reader.len().min(buffer.len() - read_total);
            if n == 0 {
                break;
            }
            buffer[read_total..read_total + n].copy_from_slice(&reader[..n]);
            reader.consume(n);
            read_total += n;
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

    #[test]
    fn writer_commits_on_drop() {
        let buf: RingBuffer<u32, 8> = RingBuffer::new();
        {
            let mut w = buf.begin_write().unwrap();
            assert_eq!(w.capacity(), 8);
            w.extend_from_slice(&[1, 2, 3]);
            assert_eq!(w.len(), 3);
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.try_pop(), Some(1));
    }

    #[test]
    fn writer_slice_write_then_commit() {
        let buf: RingBuffer<u32, 8> = RingBuffer::new();
        {
            let mut w = buf.begin_write().unwrap();
            w[0] = 7;
            w[1] = 8;
            w.commit(2);
        }
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.try_pop(), Some(7));
        assert_eq!(buf.try_pop(), Some(8));
    }

    #[test]
    fn writer_claim_is_exclusive() {
        let buf: RingBuffer<u32, 8> = RingBuffer::new();
        let w1 = buf.begin_write();
        assert!(w1.is_some());
        assert!(buf.begin_write().is_none());
        drop(w1);
        assert!(buf.begin_write().is_some());
    }

    #[test]
    fn reader_wraps_around() {
        let buf: RingBuffer<u32, 4> = RingBuffer::new();
        for i in 0..3 {
            assert!(buf.try_push(i));
        }
        assert_eq!(buf.try_pop(), Some(0));
        assert_eq!(buf.try_pop(), Some(1));
        assert!(buf.try_push(3));
        assert!(buf.try_push(4));

        let mut out = [0u32; 3];
        let mut got = 0;
        while got < 3 {
            let mut r = buf.begin_read().unwrap();
            let n = r.len().min(3 - got);
            assert!(n > 0);
            out[got..got + n].copy_from_slice(&r[..n]);
            r.consume(n);
            got += n;
        }
        assert_eq!(out, [2, 3, 4]);
        assert!(buf.is_empty());
    }

    #[test]
    fn writer_stops_at_wrap_boundary() {
        let buf: RingBuffer<u32, 4> = RingBuffer::new();
        assert!(buf.try_push(1));
        assert!(buf.try_push(2));
        assert_eq!(buf.try_pop(), Some(1));
        assert_eq!(buf.try_pop(), Some(2));
        assert!(buf.try_push(3));
        {
            let w = buf.begin_write().unwrap();
            assert_eq!(w.capacity(), 1);
        }
    }

    #[test]
    fn batch_write_read_roundtrip() {
        let buf: RingBuffer<u8, 16> = RingBuffer::new();
        let data = [10u8, 20, 30, 40, 50];
        assert_eq!(buf.write(&data), 5);
        let mut out = [0u8; 5];
        assert_eq!(buf.read(&mut out), 5);
        assert_eq!(out, data);
        assert!(buf.is_empty());
    }

    #[test]
    fn batch_write_respects_full_buffer() {
        let buf: RingBuffer<u8, 4> = RingBuffer::new();
        assert_eq!(buf.write(&[1, 2, 3, 4, 5, 6]), 4);
        assert!(buf.is_full());
        assert_eq!(buf.write(&[7]), 0);
        let mut out = [0u8; 4];
        assert_eq!(buf.read(&mut out), 4);
        assert_eq!(out, [1, 2, 3, 4]);
    }
}
