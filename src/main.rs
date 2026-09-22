#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

mod tasks;
mod sync;
mod util;
mod mem;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::TimerGroup,
};
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;
use portable_atomic::Ordering;

esp_bootloader_esp_idf::esp_app_desc!();

#[allow(unused_imports)]
use crate::util::log::*;

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(all(feature = "defmt", feature = "dev"))]
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    loop {
        unsafe { core::arch::asm!("break 1, 15") }
    }
}

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<3>> = StaticCell::new();

static MID_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

#[repr(C, align(32))]
pub struct SystemState {
    pub boot_time: u64,
    pub sensor_cycles: u32,
    pub flags: u32,
    _pad: [u8; 16],
}

impl SystemState {
    pub const fn new() -> Self {
        Self {
            boot_time: 0,
            sensor_cycles: 0,
            flags: 0,
            _pad: [0; 16],
        }
    }
}

#[link_section = ".dram.data"]
static mut SYSTEM_STATE: SystemState = SystemState::new();

#[esp_rtos::main]
async fn main(low_prio_spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    log_info!("RustRTOS v{} starting on ESP32-S3", env!("CARGO_PKG_VERSION"));

    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    esp_rtos::start(timg0.timer0);

    log_info!("esp-rtos scheduler initialized");

    unsafe {
        SYSTEM_STATE.boot_time = Instant::now().as_micros();
    }

    let high_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt3);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);

    let high_prio_spawner = high_prio_executor.start(Priority::Priority3);

    log_info!("High priority executor started (Priority3)");

    high_prio_spawner.must_spawn(tasks::critical::critical_sensor_task());

    let mid_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt2);
    let mid_prio_executor = MID_PRIO_EXECUTOR.init(mid_prio_executor);

    let mid_prio_spawner = mid_prio_executor.start(Priority::Priority2);

    log_info!("Mid priority executor started (Priority2)");

    mid_prio_spawner.must_spawn(tasks::normal::periodic_task());

    low_prio_spawner.must_spawn(tasks::normal::led_blink_task(led));
    low_prio_spawner.must_spawn(tasks::normal::background_task());

    log_info!("All tasks spawned, entering main loop");

    let mut tick_count: u64 = 0;

    loop {
        tick_count += 1;

        unsafe {
            SYSTEM_STATE.flags = tick_count as u32;
            SYSTEM_STATE.sensor_cycles = tasks::critical::SENSOR_CYCLES.load(Ordering::Relaxed);
        }

        if tick_count % 10 == 0 {
            log_info!("System heartbeat: {} ticks", tick_count);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
