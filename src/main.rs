#![no_std]
#![no_main]
#![cfg_attr(
    all(feature = "defmt", feature = "dev"),
    feature(asm_experimental_arch)
)]

use rustrtos::{apps, runtime::Runtime};

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    interrupt::software::SoftwareInterruptControl,
    timer::timg::TimerGroup,
};
use portable_atomic::Ordering;

esp_bootloader_esp_idf::esp_app_desc!();

#[allow(unused_imports)]
use rustrtos::util::log::*;

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
    loop {
        core::hint::spin_loop();
    }
}

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

    log_info!(
        "RustRTOS v{} starting on ESP32-S3",
        env!("CARGO_PKG_VERSION")
    );

    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let runtime = Runtime::start(low_prio_spawner, timg0.timer0, sw_ints);

    log_info!("RustRTOS runtime initialized");

    unsafe {
        SYSTEM_STATE.boot_time = Instant::now().as_micros();
    }

    runtime
        .spawn_high(apps::demo::critical::critical_sensor_task())
        .expect("failed to spawn high-priority task");

    runtime
        .spawn_normal(apps::demo::normal::periodic_task())
        .expect("failed to spawn normal-priority task");

    runtime
        .spawn_low(apps::demo::normal::led_blink_task(led))
        .expect("failed to spawn low-priority LED task");
    runtime
        .spawn_low(apps::demo::normal::background_task())
        .expect("failed to spawn low-priority background task");

    log_info!("All tasks spawned, entering main loop");

    let mut tick_count: u64 = 0;

    loop {
        tick_count += 1;

        unsafe {
            SYSTEM_STATE.flags = tick_count as u32;
            SYSTEM_STATE.sensor_cycles =
                apps::demo::critical::SENSOR_CYCLES.load(Ordering::Relaxed);
        }

        if tick_count % 10 == 0 {
            log_info!("System heartbeat: {} ticks", tick_count);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
