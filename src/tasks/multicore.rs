use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use esp_hal::system::Cpu;
#[cfg(feature = "multicore")]
use esp_hal::system::Stack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreId {
    Core0 = 0,
    Core1 = 1,
}

impl CoreId {
    pub fn current() -> Self {
        match Cpu::current() {
            Cpu::ProCpu => CoreId::Core0,
            Cpu::AppCpu => CoreId::Core1,
        }
    }

    pub fn other(&self) -> Self {
        match self {
            CoreId::Core0 => CoreId::Core1,
            CoreId::Core1 => CoreId::Core0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAssignment {
    Auto {
        io_on_core1: bool,
    },
    Manual(CoreId),
    Any,
}

impl Default for CoreAssignment {
    fn default() -> Self {
        CoreAssignment::Auto { io_on_core1: true }
    }
}

impl CoreAssignment {
    pub const fn auto() -> Self {
        CoreAssignment::Auto { io_on_core1: true }
    }

    pub const fn manual(core: CoreId) -> Self {
        CoreAssignment::Manual(core)
    }

    pub const fn core0() -> Self {
        CoreAssignment::Manual(CoreId::Core0)
    }

    pub const fn core1() -> Self {
        CoreAssignment::Manual(CoreId::Core1)
    }

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
            CoreAssignment::Any => CoreId::current(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    CpuIntensive,
    IoIntensive,
    Realtime,
    Background,
    General,
}

impl TaskType {
    pub fn is_io_intensive(&self) -> bool {
        matches!(self, TaskType::IoIntensive)
    }

    pub fn recommended_core(&self) -> CoreId {
        match self {
            TaskType::IoIntensive => CoreId::Core1,
            TaskType::CpuIntensive => CoreId::Core1,
            TaskType::Realtime => CoreId::Core0,
            TaskType::Background => CoreId::Core1,
            TaskType::General => CoreId::Core0,
        }
    }
}

static CORE1_STARTED: AtomicBool = AtomicBool::new(false);
static CORE1_READY: AtomicBool = AtomicBool::new(false);

pub struct Core1;

impl Core1 {
    pub fn is_started() -> bool {
        CORE1_STARTED.load(Ordering::Acquire)
    }

    pub fn is_ready() -> bool {
        CORE1_READY.load(Ordering::Acquire)
    }

    #[cfg(feature = "multicore")]
    pub fn start_with_rtos<const SIZE: usize, F>(
        cpu_ctrl: esp_hal::peripherals::CPU_CTRL<'static>,
        int0: esp_hal::interrupt::software::SoftwareInterrupt<'static, 0>,
        int1: esp_hal::interrupt::software::SoftwareInterrupt<'static, 1>,
        stack: &'static mut Stack<SIZE>,
        entry: F,
    ) where
        F: FnOnce() + Send + 'static,
    {
        if CORE1_STARTED.swap(true, Ordering::AcqRel) {
            return;
        }

        esp_rtos::start_second_core(cpu_ctrl, int0, int1, stack, move || {
            CORE1_READY.store(true, Ordering::Release);
            entry();
        });
    }

    pub fn wait_ready() {
        while !Self::is_ready() {
            core::hint::spin_loop();
        }
    }
}

/// A single-producer/single-consumer lock-free channel suitable for Core0/Core1 IPC.
///
/// The producer and consumer must each have exactly one caller. Ownership of `T` crosses
/// the cores through the release/acquire publication of the head and tail counters.
pub struct IpcChannel<T, const N: usize> {
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    _marker: PhantomData<T>,
}

impl<T, const N: usize> IpcChannel<T, N> {
    pub const fn new() -> Self {
        assert!(N > 0, "IPC channel capacity must be greater than zero");
        Self {
            buffer: UnsafeCell::new(unsafe {
                MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init()
            }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            _marker: PhantomData,
        }
    }

    pub fn try_send(&self, value: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return Err(value);
        }

        unsafe {
            self.buffer
                .get()
                .cast::<MaybeUninit<T>>()
                .add(head % N)
                .write(MaybeUninit::new(value));
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub fn try_recv(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }

        let value = unsafe {
            self.buffer
                .get()
                .cast::<MaybeUninit<T>>()
                .add(tail % N)
                .read()
                .assume_init()
        };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.len() >= N
    }

    pub fn len(&self) -> usize {
        self.head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire))
            .min(N)
    }

    pub const fn capacity(&self) -> usize {
        N
    }
}

unsafe impl<T: Send, const N: usize> Send for IpcChannel<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for IpcChannel<T, N> {}

pub struct IpcSignal {
    flag: AtomicBool,
}

impl IpcSignal {
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    pub fn signal(&self) {
        self.flag.store(true, Ordering::Release);
    }

    pub fn check_and_clear(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }

    pub fn wait(&self) {
        while !self.check_and_clear() {
            core::hint::spin_loop();
        }
    }

    pub fn try_wait(&self) -> bool {
        self.check_and_clear()
    }

    pub fn is_signaled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

pub struct IpcSemaphore {
    count: AtomicU8,
    max: u8,
}

impl IpcSemaphore {
    pub const fn new(initial: u8, max: u8) -> Self {
        Self {
            count: AtomicU8::new(initial),
            max,
        }
    }

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

    pub fn release(&self) {
        loop {
            let current = self.count.load(Ordering::Acquire);
            if current >= self.max {
                return;
            }

            if self.count
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    pub fn count(&self) -> u8 {
        self.count.load(Ordering::Relaxed)
    }

    pub const fn max(&self) -> u8 {
        self.max
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MulticoreStats {
    pub core0_active: bool,
    pub core1_started: bool,
    pub core1_ready: bool,
}

impl MulticoreStats {
    pub fn current() -> Self {
        Self {
            core0_active: true,
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
