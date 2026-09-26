//! Unified RustRTOS runtime entry points.
//!
//! The underlying scheduler is provided by `esp-rtos` and Embassy. This
//! module owns the executor lanes used by the RustRTOS system so applications
//! do not need to allocate software interrupts or executors themselves.

use embassy_executor::{SendSpawner, SpawnError, SpawnToken, Spawner};
use esp_hal::{
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::Timer,
};
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

static HIGH_PRIORITY_EXECUTOR: StaticCell<InterruptExecutor<3>> = StaticCell::new();
static NORMAL_PRIORITY_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

/// Project-owned entry point for the ESP32-S3 async runtime.
///
/// `esp-rtos` still provides the scheduler. `Runtime` owns the RustRTOS
/// mapping from task priority lanes to executor instances and the software
/// interrupts reserved for those lanes.
#[derive(Clone, Copy)]
pub struct Runtime {
    low: Spawner,
    normal: SendSpawner,
    high: SendSpawner,
}

impl Runtime {
    /// Starts the base scheduler and the RustRTOS priority executors.
    ///
    /// Software interrupts 2 and 3 are reserved for the normal and high
    /// priority lanes. Interrupts 0 and 1 remain available for the optional
    /// second-core startup path.
    pub fn start(
        low: Spawner,
        timer: Timer<'static>,
        software_interrupts: SoftwareInterruptControl<'static>,
    ) -> Self {
        esp_rtos::start(timer);

        let high = HIGH_PRIORITY_EXECUTOR
            .init(InterruptExecutor::new(
                software_interrupts.software_interrupt3,
            ))
            .start(Priority::Priority3);
        let normal = NORMAL_PRIORITY_EXECUTOR
            .init(InterruptExecutor::new(
                software_interrupts.software_interrupt2,
            ))
            .start(Priority::Priority2);

        Self { low, normal, high }
    }

    /// Returns the base scheduler spawner.
    pub fn low(&self) -> Spawner {
        self.low
    }

    /// Returns the normal-priority spawner.
    pub fn normal(&self) -> SendSpawner {
        self.normal
    }

    /// Returns the high-priority spawner.
    pub fn high(&self) -> SendSpawner {
        self.high
    }

    /// Spawns a task on the base scheduler.
    pub fn spawn_low<S>(&self, token: SpawnToken<S>) -> Result<(), SpawnError> {
        self.low.spawn(token)
    }

    /// Spawns a task on the normal-priority executor.
    pub fn spawn_normal<S: Send>(&self, token: SpawnToken<S>) -> Result<(), SpawnError> {
        self.normal.spawn(token)
    }

    /// Spawns a task on the high-priority executor.
    pub fn spawn_high<S: Send>(&self, token: SpawnToken<S>) -> Result<(), SpawnError> {
        self.high.spawn(token)
    }
}
