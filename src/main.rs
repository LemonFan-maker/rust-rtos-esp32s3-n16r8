#![no_std]
#![no_main]
#![cfg_attr(
    all(feature = "defmt", feature = "dev"),
    feature(asm_experimental_arch)
)]

use rustrtos::watchdog::{supervised_feed, Supervisor, Watchdog};
use rustrtos::{
    apps,
    runtime::{start_priority_executors, start_scheduler},
    system::SystemState,
};

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

static SYSTEM_STATE: SystemState = SystemState::new();
static SUPERVISOR: Supervisor<4> = Supervisor::new();

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
    start_scheduler(timg0.timer0);
    let (high_priority_spawner, normal_priority_spawner) = start_priority_executors(sw_ints);

    log_info!("RustRTOS runtime initialized");
    let mut watchdog = Watchdog::enable(peripherals.LPWR, 3_000);
    let sensor_heartbeat = SUPERVISOR
        .register(1_000)
        .expect("sensor heartbeat slot available");
    let processing_heartbeat = SUPERVISOR
        .register(1_000)
        .expect("processing heartbeat slot available");
    let led_heartbeat = SUPERVISOR
        .register(1_500)
        .expect("LED heartbeat slot available");
    let background_heartbeat = SUPERVISOR
        .register(2_500)
        .expect("background heartbeat slot available");

    SYSTEM_STATE.set_boot_time(Instant::now().as_micros());

    high_priority_spawner
        .spawn(apps::demo::critical::critical_sensor_task(sensor_heartbeat))
        .expect("failed to spawn high-priority task");

    normal_priority_spawner
        .spawn(apps::demo::normal::periodic_task(processing_heartbeat))
        .expect("failed to spawn normal-priority task");

    low_prio_spawner
        .spawn(apps::demo::normal::led_blink_task(led, led_heartbeat))
        .expect("failed to spawn low-priority LED task");
    low_prio_spawner
        .spawn(apps::demo::normal::background_task(background_heartbeat))
        .expect("failed to spawn low-priority background task");

    log_info!("All tasks spawned, entering main loop");

    let mut tick_count: u64 = 0;

    loop {
        tick_count += 1;

        SYSTEM_STATE.update(
            tick_count,
            apps::demo::critical::SENSOR_CYCLES.load(Ordering::Relaxed),
        );

        if tick_count % 10 == 0 {
            log_info!("System heartbeat: {} ticks", tick_count);
        }
        if !supervised_feed(&mut watchdog, &SUPERVISOR) {
            log_warn!("watchdog feed withheld: supervised task heartbeat expired");
        }

        Timer::after(Duration::from_millis(500)).await;
    }
}
