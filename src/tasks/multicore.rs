//! 双核调度模块
//!
//! 提供 ESP32-S3 双核 (Core0 + Core1) 的任务调度支持。
//! 支持两种模式:
//! - 自动分配: 按任务类型自动分配核心
//! - 手动分配: 用户显式指定运行核心
//!
//! # 核心分配策略
//!
//! 自动模式默认策略:
//! - Core0 (PRO_CPU): 主逻辑、UI、网络协议栈
//! - Core1 (APP_CPU): IO 密集型、传感器采样、计算密集型
//!
//! # 示例
//!
//! ```rust,ignore
//! use rustrtos::tasks::multicore::{Core1, CoreAssignment, IpcChannel};
//!
//! // 启动 Core1
//! Core1::start(8192, || {
//!     // Core1 入口代码
//!     let executor = Executor::new();
//!     executor.run(|spawner| {
//!         spawner.spawn(sensor_task()).ok();
//!     });
//! });
//!
//! // 核间通信
//! static IPC: IpcChannel<SensorData, 16> = IpcChannel::new();
//! IPC.send(data); // Core1
//! let data = IPC.recv().await; // Core0
//! ```

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use esp_hal::system::{Cpu, Stack};
use heapless::spsc::Queue;

/// CPU 核心标识
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreId {
    /// Core0 (PRO_CPU) - 主核心
    Core0 = 0,
    /// Core1 (APP_CPU) - 应用核心
    Core1 = 1,
}

impl CoreId {
    /// 获取当前运行的核心
    pub fn current() -> Self {
        match Cpu::current() {
            Cpu::ProCpu => CoreId::Core0,
            #[cfg(multi_core)]
            Cpu::AppCpu => CoreId::Core1,
            #[cfg(not(multi_core))]
            _ => CoreId::Core0,
        }
    }
    
    /// 获取另一个核心
    pub fn other(&self) -> Self {
        match self {
            CoreId::Core0 => CoreId::Core1,
            CoreId::Core1 => CoreId::Core0,
        }
    }
}

/// 核心分配策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAssignment {
    /// 自动分配: 根据任务类型决定
    Auto {
        /// IO 密集型任务是否分配到 Core1
        io_on_core1: bool,
    },
    /// 手动指定核心
    Manual(CoreId),
    /// 任意核心 (调度器决定)
    Any,
}

impl Default for CoreAssignment {
    fn default() -> Self {
        CoreAssignment::Auto { io_on_core1: true }
    }
}

impl CoreAssignment {
    /// 创建自动分配策略 (默认 IO 密集型任务在 Core1)
    pub const fn auto() -> Self {
        CoreAssignment::Auto { io_on_core1: true }
    }
    
    /// 创建手动分配策略
    pub const fn manual(core: CoreId) -> Self {
        CoreAssignment::Manual(core)
    }
    
    /// 强制 Core0
    pub const fn core0() -> Self {
        CoreAssignment::Manual(CoreId::Core0)
    }
    
    /// 强制 Core1
    pub const fn core1() -> Self {
        CoreAssignment::Manual(CoreId::Core1)
    }
    
    /// 解析为目标核心
    pub fn resolve(&self, is_io_intensive: bool) -> CoreId {
        match self {
            CoreAssignment::Auto { io_on_core1 } => {
                if is_io_intensive && *io_on_core1 {
                    CoreId::Core1
                } else {
                    CoreId::Core0
                }
            }
            CoreAssignment::Manual(core) => *core,
            CoreAssignment::Any => CoreId::current(), // 当前核心
        }
    }
}

/// 任务类型 (用于自动分配)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// CPU 密集型 (计算、加密)
    CpuIntensive,
    /// IO 密集型 (传感器、外设通信)
    IoIntensive,
    /// 实时任务 (需要低延迟)
    Realtime,
    /// 后台任务 (低优先级)
    Background,
    /// 通用任务
    General,
}

impl TaskType {
    /// 是否为 IO 密集型
    pub fn is_io_intensive(&self) -> bool {
        matches!(self, TaskType::IoIntensive)
    }
    
    /// 推荐的核心
    pub fn recommended_core(&self) -> CoreId {
        match self {
            TaskType::IoIntensive => CoreId::Core1,
            TaskType::CpuIntensive => CoreId::Core1,
            TaskType::Realtime => CoreId::Core0, // 实时任务在主核心
            TaskType::Background => CoreId::Core1,
            TaskType::General => CoreId::Core0,
        }
    }
}

/// Core1 状态
static CORE1_STARTED: AtomicBool = AtomicBool::new(false);
static CORE1_READY: AtomicBool = AtomicBool::new(false);

/// Core1 管理器
pub struct Core1;

impl Core1 {
    /// 检查 Core1 是否已启动
    pub fn is_started() -> bool {
        CORE1_STARTED.load(Ordering::Acquire)
    }
    
    /// 检查 Core1 是否就绪
    pub fn is_ready() -> bool {
        CORE1_READY.load(Ordering::Acquire)
    }
    
    /// 启动 Core1
    ///
    /// 使用 esp-rtos 的 `start_second_core` 函数启动第二个核心。
    ///
    /// # 参数
    ///
    /// - `cpu_ctrl`: CPU 控制外设
    /// - `sw_int`: 软件中断 (用于核间通信)
    /// - `stack`: Core1 的栈空间
    /// - `entry`: Core1 入口函数
    ///
    /// # 示例
    ///
    /// ```rust,ignore
    /// use esp_hal::system::Stack;
    /// use static_cell::StaticCell;
    ///
    /// static STACK: StaticCell<Stack<8192>> = StaticCell::new();
    /// let stack = STACK.init(Stack::new());
    ///
    /// Core1::start_with_rtos(
    ///     peripherals.CPU_CTRL,
    ///     sw_ints.software_interrupt1,
    ///     stack,
    ///     || {
    ///         // Core1 代码
    ///     },
    /// );
    /// ```
    #[cfg(feature = "multicore")]
    pub fn start_with_rtos<const SIZE: usize, F>(
        cpu_ctrl: esp_hal::peripherals::CPU_CTRL<'static>,
        sw_int: esp_hal::interrupt::software::SoftwareInterrupt<'static, 1>,
        stack: &'static mut Stack<SIZE>,
        entry: F,
    ) where
        F: FnOnce() + Send + 'static,
    {
        if CORE1_STARTED.swap(true, Ordering::AcqRel) {
            // 已经启动过
            return;
        }
        
        esp_rtos::start_second_core(cpu_ctrl, sw_int, stack, move || {
            CORE1_READY.store(true, Ordering::Release);
            entry();
        });
    }
    
    /// 等待 Core1 就绪
    pub fn wait_ready() {
        while !Self::is_ready() {
            core::hint::spin_loop();
        }
    }
}

/// 核间通信通道
///
/// 基于 SPSC 无锁队列实现的核间通信。
/// 
/// # 类型参数
///
/// - `T`: 消息类型
/// - `N`: 队列容量
pub struct IpcChannel<T, const N: usize> {
    queue: UnsafeCell<Queue<T, N>>,
    _marker: PhantomData<T>,
}

impl<T, const N: usize> IpcChannel<T, N> {
    /// 创建新的 IPC 通道
    pub const fn new() -> Self {
        Self {
            queue: UnsafeCell::new(Queue::new()),
            _marker: PhantomData,
        }
    }
    
    /// 发送消息 (非阻塞)
    ///
    /// # 返回
    ///
    /// - `Ok(())`: 发送成功
    /// - `Err(value)`: 队列已满，返回未发送的值
    pub fn try_send(&self, value: T) -> Result<(), T> {
        let queue = unsafe { &mut *self.queue.get() };
        queue.enqueue(value)
    }
    
    /// 接收消息 (非阻塞)
    ///
    /// # 返回
    ///
    /// - `Some(value)`: 接收成功
    /// - `None`: 队列为空
    pub fn try_recv(&self) -> Option<T> {
        let queue = unsafe { &mut *self.queue.get() };
        queue.dequeue()
    }
    
    /// 检查队列是否为空
    pub fn is_empty(&self) -> bool {
        let queue = unsafe { &*self.queue.get() };
        queue.is_empty()
    }
    
    /// 检查队列是否已满
    pub fn is_full(&self) -> bool {
        let queue = unsafe { &*self.queue.get() };
        queue.is_full()
    }
    
    /// 获取队列中的消息数量
    pub fn len(&self) -> usize {
        let queue = unsafe { &*self.queue.get() };
        queue.len()
    }
    
    /// 获取队列容量
    pub const fn capacity(&self) -> usize {
        N
    }
}

// Safety: IpcChannel 使用 SPSC 队列，一个核心发送，另一个核心接收
unsafe impl<T: Send, const N: usize> Send for IpcChannel<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for IpcChannel<T, N> {}

/// 核间信号
///
/// 简单的二进制信号，用于核间同步。
pub struct IpcSignal {
    flag: AtomicBool,
}

impl IpcSignal {
    /// 创建新的信号
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }
    
    /// 发送信号
    pub fn signal(&self) {
        self.flag.store(true, Ordering::Release);
    }
    
    /// 检查并清除信号
    pub fn check_and_clear(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }
    
    /// 等待信号 (忙等待)
    pub fn wait(&self) {
        while !self.check_and_clear() {
            core::hint::spin_loop();
        }
    }
    
    /// 尝试等待信号 (非阻塞)
    pub fn try_wait(&self) -> bool {
        self.check_and_clear()
    }
    
    /// 检查信号是否已设置
    pub fn is_signaled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// 核间计数信号量
pub struct IpcSemaphore {
    count: AtomicU8,
    max: u8,
}

impl IpcSemaphore {
    /// 创建新的信号量
    pub const fn new(initial: u8, max: u8) -> Self {
        Self {
            count: AtomicU8::new(initial),
            max,
        }
    }
    
    /// 获取信号量 (非阻塞)
    pub fn try_acquire(&self) -> bool {
        loop {
            let current = self.count.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            
            if self.count
                .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
    
    /// 释放信号量
    pub fn release(&self) {
        loop {
            let current = self.count.load(Ordering::Acquire);
            if current >= self.max {
                return; // 已达最大值
            }
            
            if self.count
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }
    
    /// 获取当前计数
    pub fn count(&self) -> u8 {
        self.count.load(Ordering::Relaxed)
    }
    
    /// 获取最大值
    pub const fn max(&self) -> u8 {
        self.max
    }
}

/// 双核统计信息
#[derive(Debug, Clone, Copy)]
pub struct MulticoreStats {
    /// Core0 是否活跃
    pub core0_active: bool,
    /// Core1 是否启动
    pub core1_started: bool,
    /// Core1 是否就绪
    pub core1_ready: bool,
}

impl MulticoreStats {
    /// 获取当前统计
    pub fn current() -> Self {
        Self {
            core0_active: true, // Core0 总是活跃
            core1_started: Core1::is_started(),
            core1_ready: Core1::is_ready(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_core_assignment_default() {
        let assignment = CoreAssignment::default();
        assert_eq!(assignment, CoreAssignment::Auto { io_on_core1: true });
    }
    
    #[test]
    fn test_core_assignment_resolve() {
        let auto = CoreAssignment::auto();
        assert_eq!(auto.resolve(true), CoreId::Core1);
        assert_eq!(auto.resolve(false), CoreId::Core0);
        
        let manual = CoreAssignment::manual(CoreId::Core1);
        assert_eq!(manual.resolve(false), CoreId::Core1);
    }
    
    #[test]
    fn test_task_type_recommendation() {
        assert_eq!(TaskType::IoIntensive.recommended_core(), CoreId::Core1);
        assert_eq!(TaskType::Realtime.recommended_core(), CoreId::Core0);
    }
}
