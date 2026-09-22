#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_hal::{
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::TimerGroup,
};
use esp_rtos::embassy::InterruptExecutor;
use portable_atomic::{AtomicU32, Ordering};
use static_cell::StaticCell;

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

static HIGH_PRIO_COUNT: AtomicU32 = AtomicU32::new(0);
static DATA_SIGNAL: Signal<CriticalSectionRawMutex, u32> = Signal::new();
static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

#[embassy_executor::task]
async fn high_priority_task() {
    println!("High priority task started");

    loop {
        let count = HIGH_PRIO_COUNT.fetch_add(1, Ordering::Relaxed);

        if count % 100 == 0 {
            DATA_SIGNAL.signal(count);
        }

        Timer::after(Duration::from_micros(100)).await;
    }
}

#[embassy_executor::task]
async fn medium_priority_task() {
    println!("Medium priority task started");

    loop {
        let count = DATA_SIGNAL.wait().await;
        println!("Received data: {}", count);

        Timer::after(Duration::from_millis(10)).await;
    }
}

#[embassy_executor::task]
async fn background_task() {
    println!("Background task started");

    loop {
        Timer::after(Duration::from_secs(5)).await;
        let count = HIGH_PRIO_COUNT.load(Ordering::Relaxed);
        println!("Background: total samples = {}", count);
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Multi-priority example starting");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    let high_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt2);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);
    let high_prio_spawner = high_prio_executor.start(Priority::Priority3);

    high_prio_spawner.spawn(high_priority_task()).ok();
    spawner.spawn(medium_priority_task()).ok();
    spawner.spawn(background_task()).ok();

    println!("All tasks spawned");

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
