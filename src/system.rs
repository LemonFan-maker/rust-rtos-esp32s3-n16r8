//! Atomic system diagnostics state shared by the RustRTOS runtime.
//!
//! The state is intentionally a set of independent atomic counters. Readers
//! never need a critical section, and updates do not expose a mutable global
//! object to application tasks.

use portable_atomic::{AtomicU32, AtomicU64, Ordering};

/// Point-in-time copy of the diagnostics counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemSnapshot {
    /// Boot timestamp in microseconds from the Embassy time base.
    pub boot_time_us: u64,
    /// Number of completed runtime heartbeat iterations.
    pub tick_count: u64,
    /// Number of samples produced by the critical application task.
    pub sensor_cycles: u32,
}

/// Runtime diagnostics state with lock-free atomic updates.
pub struct SystemState {
    boot_time_us: AtomicU64,
    tick_count: AtomicU64,
    sensor_cycles: AtomicU32,
}

impl SystemState {
    /// Creates an empty diagnostics state.
    pub const fn new() -> Self {
        Self {
            boot_time_us: AtomicU64::new(0),
            tick_count: AtomicU64::new(0),
            sensor_cycles: AtomicU32::new(0),
        }
    }

    /// Records the system boot timestamp.
    #[inline]
    pub fn set_boot_time(&self, boot_time_us: u64) {
        self.boot_time_us.store(boot_time_us, Ordering::Release);
    }

    /// Publishes the latest runtime counters.
    #[inline]
    pub fn update(&self, tick_count: u64, sensor_cycles: u32) {
        self.tick_count.store(tick_count, Ordering::Relaxed);
        self.sensor_cycles.store(sensor_cycles, Ordering::Relaxed);
    }

    /// Reads a diagnostics snapshot.
    #[inline]
    pub fn snapshot(&self) -> SystemSnapshot {
        SystemSnapshot {
            boot_time_us: self.boot_time_us.load(Ordering::Acquire),
            tick_count: self.tick_count.load(Ordering::Relaxed),
            sensor_cycles: self.sensor_cycles.load(Ordering::Relaxed),
        }
    }
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_zeroed() {
        let state = SystemState::new();
        assert_eq!(
            state.snapshot(),
            SystemSnapshot {
                boot_time_us: 0,
                tick_count: 0,
                sensor_cycles: 0,
            }
        );
    }

    #[test]
    fn update_publishes_runtime_counters() {
        let state = SystemState::new();
        state.set_boot_time(1234);
        state.update(8, 42);
        assert_eq!(
            state.snapshot(),
            SystemSnapshot {
                boot_time_us: 1234,
                tick_count: 8,
                sensor_cycles: 42,
            }
        );
    }
}
