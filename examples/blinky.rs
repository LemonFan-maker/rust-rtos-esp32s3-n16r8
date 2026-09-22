#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    timer::timg::TimerGroup,
};

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

#[embassy_executor::task]
async fn blink_task(mut led: Output<'static>) {
    println!("Blink task started");

    let mut count: u32 = 0;

    loop {
        led.set_high();
        println!("LED ON (count: {})", count);
        Timer::after(Duration::from_millis(500)).await;

        led.set_low();
        println!("LED OFF");
        Timer::after(Duration::from_millis(500)).await;

        count += 1;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Blinky example starting on ESP32-S3 @ 240MHz");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    println!("Spawning blink task...");

    spawner.spawn(blink_task(led)).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
