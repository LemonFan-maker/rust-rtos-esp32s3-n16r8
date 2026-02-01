//! 零拷贝环形缓冲区
//!
//! 高性能单生产者单消费者 (SPSC) 环形缓冲区
//! 特点:
//! - 零拷贝读写 (返回切片引用)
//! - 无锁实现 (使用原子操作)
//! - 缓存友好的内存布局
//! - 编译时确定容量

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use portable_atomic::{AtomicUsize, Ordering};

/// 零拷贝环形缓冲区
///
/// 单生产者单消费者 (SPSC) 设计，无需锁
///
/// # Type Parameters
/// * `T` - 元素类型
/// * `N` - 容量 (必须是 2 的幂以优化取模运算)
///
/// # Example
/// ```ignore
/// static mut BUFFER: RingBuffer<u8, 256> = RingBuffer::new();
///
/// // 生产者
/// unsafe {
///     let slice = BUFFER.write_slice();
///     slice[0..4].copy_from_slice(&[1, 2, 3, 4]);
///     BUFFER.commit_write(4);
/// }
///
/// // 消费者
/// unsafe {
///     let slice = BUFFER.read_slice();
///     process(slice);
///     BUFFER.commit_read(slice.len());
/// }
/// ```
#[repr(C, align(32))] // 缓存行对齐
pub struct RingBuffer<T, const N: usize> {
    /// 数据存储
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
    /// 写入位置 (生产者更新)
    head: AtomicUsize,
    /// 读取位置 (消费者更新)
    tail: AtomicUsize,
    /// 填充到缓存行避免 false sharing
    _pad: [u8; 16],
}

// Safety: RingBuffer 在 SPSC 场景下是线程安全的
unsafe impl<T: Send, const N: usize> Send for RingBuffer<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for RingBuffer<T, N> {}

impl<T, const N: usize> RingBuffer<T, N> {
    /// 创建新的空环形缓冲区
    ///
    /// # Panics
    /// 编译时检查 N 必须是 2 的幂
    pub const fn new() -> Self {
        // 编译时检查: N 必须是 2 的幂
        assert!(N > 0 && (N & (N - 1)) == 0, "N must be a power of 2");
        
        Self {
            buffer: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            _pad: [0; 16],
        }
    }
    
    /// 缓冲区容量
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }
    
    /// 当前元素数量
    #[inline(always)]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }
    
    /// 是否为空
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    
    /// 是否已满
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len() >= N
    }
    
    /// 可写入的空间大小
    #[inline(always)]
    pub fn available_write(&self) -> usize {
        N - self.len()
    }
    
    /// 可读取的数据大小
    #[inline(always)]
    pub fn available_read(&self) -> usize {
        self.len()
    }
    
    /// 掩码 (用于快速取模)
    #[inline(always)]
    const fn mask(&self) -> usize {
        N - 1
    }
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    /// 获取可写入的连续切片 (零拷贝)
    ///
    /// # Safety
    /// - 只能由单个生产者调用
    /// - 写入后必须调用 `commit_write`
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
        
        // 计算连续可写区域
        let contiguous = if head_idx >= tail_idx {
            // head 在 tail 后面或相等
            N - head_idx
        } else {
            // head 回绕到 tail 前面
            tail_idx - head_idx
        }.min(available);
        
        let ptr = (*self.buffer.get()).as_mut_ptr().add(head_idx) as *mut T;
        core::slice::from_raw_parts_mut(ptr, contiguous)
    }
    
    /// 提交写入
    ///
    /// # Arguments
    /// * `len` - 实际写入的字节数
    ///
    /// # Safety
    /// `len` 不能超过 `write_slice` 返回的切片长度
    #[inline(always)]
    pub unsafe fn commit_write(&self, len: usize) {
        let head = self.head.load(Ordering::Relaxed);
        self.head.store(head.wrapping_add(len), Ordering::Release);
    }
    
    /// 获取可读取的连续切片 (零拷贝)
    ///
    /// # Safety
    /// - 只能由单个消费者调用
    /// - 读取后必须调用 `commit_read`
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
        
        // 计算连续可读区域
        let contiguous = if head_idx > tail_idx {
            head_idx - tail_idx
        } else {
            N - tail_idx
        }.min(available);
        
        let ptr = (*self.buffer.get()).as_ptr().add(tail_idx) as *const T;
        core::slice::from_raw_parts(ptr, contiguous)
    }
    
    /// 提交读取
    ///
    /// # Arguments
    /// * `len` - 实际读取的字节数
    ///
    /// # Safety
    /// `len` 不能超过 `read_slice` 返回的切片长度
    #[inline(always)]
    pub unsafe fn commit_read(&self, len: usize) {
        let tail = self.tail.load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(len), Ordering::Release);
    }
    
    /// 尝试写入单个元素
    ///
    /// # Returns
    /// - `true`: 写入成功
    /// - `false`: 缓冲区已满
    #[inline]
    pub fn try_push(&self, value: T) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        
        if head.wrapping_sub(tail) >= N {
            return false; // 已满
        }
        
        let idx = head & self.mask();
        unsafe {
            let ptr = (*self.buffer.get()).as_mut_ptr().add(idx);
            (ptr as *mut T).write(value);
        }
        
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }
    
    /// 尝试读取单个元素
    ///
    /// # Returns
    /// - `Some(T)`: 读取成功
    /// - `None`: 缓冲区为空
    #[inline]
    pub fn try_pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        
        if head == tail {
            return None; // 为空
        }
        
        let idx = tail & self.mask();
        let value = unsafe {
            let ptr = (*self.buffer.get()).as_ptr().add(idx);
            (ptr as *const T).read()
        };
        
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }
    
    /// 清空缓冲区
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

// ===== 特化版本: 字节缓冲区 =====

/// 字节环形缓冲区类型别名
pub type ByteRingBuffer<const N: usize> = RingBuffer<u8, N>;

/// 256 字节环形缓冲区
pub type RingBuffer256 = RingBuffer<u8, 256>;

/// 512 字节环形缓冲区
pub type RingBuffer512 = RingBuffer<u8, 512>;

/// 1024 字节环形缓冲区
pub type RingBuffer1K = RingBuffer<u8, 1024>;

/// 4096 字节环形缓冲区
pub type RingBuffer4K = RingBuffer<u8, 4096>;

// ===== 扩展方法 =====

impl<const N: usize> RingBuffer<u8, N> {
    /// 批量写入数据
    ///
    /// # Returns
    /// 实际写入的字节数
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
    
    /// 批量读取数据
    ///
    /// # Returns
    /// 实际读取的字节数
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
        
        // Push
        assert!(buf.try_push(1));
        assert!(buf.try_push(2));
        assert!(buf.try_push(3));
        
        assert_eq!(buf.len(), 3);
        
        // Pop
        assert_eq!(buf.try_pop(), Some(1));
        assert_eq!(buf.try_pop(), Some(2));
        assert_eq!(buf.len(), 1);
        
        // Clear
        buf.clear();
        assert!(buf.is_empty());
    }
}
