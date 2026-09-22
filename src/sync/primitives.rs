use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    signal::Signal,
    channel::Channel,
    mutex::Mutex,
    watch::Watch,
    pubsub::PubSubChannel,
};

pub type CriticalSignal<T> = Signal<CriticalSectionRawMutex, T>;

pub type CriticalChannel<T, const N: usize> = Channel<CriticalSectionRawMutex, T, N>;

pub type CriticalMutex<T> = Mutex<CriticalSectionRawMutex, T>;

pub type CriticalWatch<T, const N: usize> = Watch<CriticalSectionRawMutex, T, N>;

pub type CriticalPubSub<T, const CAP: usize, const SUBS: usize, const PUBS: usize> =
    PubSubChannel<CriticalSectionRawMutex, T, CAP, SUBS, PUBS>;

#[inline]
pub const fn new_signal<T>() -> CriticalSignal<T> {
    Signal::new()
}

#[inline]
pub const fn new_channel<T, const N: usize>() -> CriticalChannel<T, N> {
    Channel::new()
}

#[inline]
pub const fn new_mutex<T>(value: T) -> CriticalMutex<T> {
    Mutex::new(value)
}

#[inline]
pub fn with_critical_section<R, F>(f: F) -> R
where
    F: FnOnce(critical_section::CriticalSection) -> R,
{
    critical_section::with(f)
}

use portable_atomic::{AtomicBool, AtomicU64, Ordering};

pub struct AtomicFlag {
    flag: AtomicBool,
}

impl AtomicFlag {
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    #[inline(always)]
    pub fn set(&self) {
        self.flag.store(true, Ordering::Release);
    }

    #[inline(always)]
    pub fn clear(&self) {
        self.flag.store(false, Ordering::Release);
    }

    #[inline(always)]
    pub fn take(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }

    #[inline(always)]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

pub struct AtomicCounter {
    count: AtomicU64,
}

impl AtomicCounter {
    pub const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }

    pub const fn with_value(value: u64) -> Self {
        Self {
            count: AtomicU64::new(value),
        }
    }

    #[inline(always)]
    pub fn increment(&self) -> u64 {
        self.count.fetch_add(1, Ordering::Relaxed) + 1
    }

    #[inline(always)]
    pub fn add(&self, value: u64) -> u64 {
        self.count.fetch_add(value, Ordering::Relaxed) + value
    }

    #[inline(always)]
    pub fn get(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

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
