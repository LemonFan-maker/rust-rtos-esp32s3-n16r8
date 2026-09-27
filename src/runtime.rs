//! Unified RustRTOS runtime entry points.
//!
//! `esp-rtos` provides the base scheduler. This module owns the RustRTOS
//! mapping from software interrupts to priority-specific Embassy executors.

use embassy_executor::SendSpawner;
use esp_hal::{
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::Timer,
};
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

static HIGH_PRIORITY_EXECUTOR: StaticCell<InterruptExecutor<3>> = StaticCell::new();
static NORMAL_PRIORITY_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

/// Starts the base `esp-rtos` scheduler on the supplied timer.
pub fn start_scheduler(timer: Timer<'static>) {
    esp_rtos::start(timer);
}

/// Starts the RustRTOS priority executor lanes.
///
/// The returned tuple is `(high_priority, normal_priority)`. Software
/// interrupts 2 and 3 are reserved for these lanes; interrupts 0 and 1
/// remain available for the optional second-core startup path.
///
/// The base scheduler must be started with [`start_scheduler`] first.
pub fn start_priority_executors(
    software_interrupts: SoftwareInterruptControl<'static>,
) -> (SendSpawner, SendSpawner) {
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

    (high, normal)
}
