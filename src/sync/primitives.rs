//! 同步原语封装
//!
//! 基于 embassy-sync 提供的同步原语，统一使用 CriticalSectionRawMutex
//! 以确保在 ESP32-S3 单核/双核环境下的正确性

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    signal::Signal,
    channel::Channel,
    mutex::Mutex,
    watch::Watch,
    pubsub::PubSubChannel,
};

// ===== 类型别名: 简化使用 =====

/// 临界区信号量 - 用于任务间单值通知
///
/// 发送方可以发送一个值，接收方异步等待
/// 多次发送只保留最后一个值
///
/// # Example
/// ```ignore
/// static SIGNAL: CriticalSignal<u32> = CriticalSignal::new();
///
/// // 发送方
/// SIGNAL.signal(42);
///
/// // 接收方 (异步)
/// let value = SIGNAL.wait().await;
/// ```
pub type CriticalSignal<T> = Signal<CriticalSectionRawMutex, T>;

/// 临界区通道 - MPMC 消息队列
///
/// 支持多发送者多接收者，固定容量
///
/// # Type Parameters
/// * `T` - 消息类型
/// * `N` - 队列容量
///
/// # Example
/// ```ignore
/// static CHANNEL: CriticalChannel<Command, 8> = CriticalChannel::new();
///
/// // 发送方 (异步，队列满时等待)
/// CHANNEL.send(Command::Start).await;
///
/// // 接收方 (异步)
/// let cmd = CHANNEL.receive().await;
/// ```
pub type CriticalChannel<T, const N: usize> = Channel<CriticalSectionRawMutex, T, N>;

/// 临界区互斥锁 - 异步互斥访问
///
/// 保护共享资源的异步访问
///
/// # Example
/// ```ignore
/// static SHARED: CriticalMutex<SharedData> = CriticalMutex::new(SharedData::new());
///
/// // 异步获取锁
/// {
///     let mut guard = SHARED.lock().await;
///     guard.value += 1;
/// } // 自动释放锁
/// ```
pub type CriticalMutex<T> = Mutex<CriticalSectionRawMutex, T>;

/// 观察者 - 广播最新值给所有订阅者
///
/// # Type Parameters
/// * `T` - 值类型 (必须实现 Clone)
/// * `N` - 最大观察者数量
///
/// # Example
/// ```ignore
/// static WATCH: CriticalWatch<SensorData, 4> = CriticalWatch::new();
///
/// // 发送方: 更新值
/// WATCH.sender().send(new_data);
///
/// // 接收方: 获取观察者并等待变化
/// let mut receiver = WATCH.receiver().unwrap();
/// let data = receiver.changed().await;
/// ```
pub type CriticalWatch<T, const N: usize> = Watch<CriticalSectionRawMutex, T, N>;

/// 发布订阅通道 - 一对多消息广播
///
/// # Type Parameters
/// * `T` - 消息类型 (必须实现 Clone)
/// * `CAP` - 缓冲区容量
/// * `SUBS` - 最大订阅者数量
/// * `PUBS` - 最大发布者数量
pub type CriticalPubSub<T, const CAP: usize, const SUBS: usize, const PUBS: usize> = 
    PubSubChannel<CriticalSectionRawMutex, T, CAP, SUBS, PUBS>;

// ===== 便捷构造函数 =====

/// 创建新的信号量
#[inline]
pub const fn new_signal<T>() -> CriticalSignal<T> {
    Signal::new()
}

/// 创建新的通道
#[inline]
pub const fn new_channel<T, const N: usize>() -> CriticalChannel<T, N> {
    Channel::new()
}

/// 创建新的互斥锁
#[inline]
pub const fn new_mutex<T>(value: T) -> CriticalMutex<T> {
    Mutex::new(value)
}

// ===== 同步工具函数 =====

/// 在临界区中执行闭包
///
/// 禁用中断确保原子性，适用于非常短的操作
///
/// # Warning
/// 临界区内不能执行任何异步操作或长时间计算
///
/// # Example
/// ```ignore
/// let value = with_critical_section(|_cs| {
///     // 原子操作
///     unsafe { SHARED_DATA += 1 }
/// });
/// ```
#[inline]
pub fn with_critical_section<R, F>(f: F) -> R
where
    F: FnOnce(critical_section::CriticalSection) -> R,
{
    critical_section::with(f)
}

// ===== 优化的原子操作封装 =====

use portable_atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// 原子标志 - 最快的任务间通知
///
/// 比 Signal 更轻量，适用于简单的布尔状态通知
pub struct AtomicFlag {
    flag: AtomicBool,
}

impl AtomicFlag {
    /// 创建新的原子标志
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }
    
    /// 设置标志
    #[inline(always)]
    pub fn set(&self) {
        self.flag.store(true, Ordering::Release);
    }
    
    /// 清除标志
    #[inline(always)]
    pub fn clear(&self) {
        self.flag.store(false, Ordering::Release);
    }
    
    /// 检查并清除标志 (test-and-clear)
    #[inline(always)]
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }
    
    /// 检查标志 (不清除)
    #[inline(always)]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// 原子计数器 - 用于统计和序列号
pub struct AtomicCounter {
    count: AtomicU64,
}

impl AtomicCounter {
    /// 创建新的计数器
    pub const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }
    
    /// 创建指定初始值的计数器
    pub const fn with_value(value: u64) -> Self {
        Self {
            count: AtomicU64::new(value),
        }
    }
    
    /// 增加并返回新值
    #[inline(always)]
    pub fn increment(&self) -> u64 {
        self.count.fetch_add(1, Ordering::Relaxed) + 1
    }
    
    /// 增加指定值并返回新值
    #[inline(always)]
    pub fn add(&self, value: u64) -> u64 {
        self.count.fetch_add(value, Ordering::Relaxed) + value
    }
    
    /// 获取当前值
    #[inline(always)]
    pub fn get(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
    
    /// 重置为 0
    #[inline(always)]
    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
    }
}

impl Default for AtomicFlag {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for AtomicCounter {
    fn default() -> Self {
        Self::new()
    }
}
