#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {};
}

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

#[embassy_executor::task]
async fn psram_demo_task() {
    println!("PSRAM Demo Task Started");

    let stats = rustrtos::mem::psram::stats();
    println!("PSRAM Stats:");
    println!("Total: {} bytes", stats.total);
    println!("Used: {} bytes", stats.used);
    println!("Free: {} bytes", stats.free);

    println!("Trying to allocate large array in PSRAM...");

    match rustrtos::mem::psram::alloc_array::<u32, 1024>() {
        Ok(mut array) => {
            println!("Allocated 1024 x u32 = 4KB in PSRAM");

            for i in 0..1024 {
                array[i] = i as u32;
            }

            let sum: u32 = array.iter().sum();
            println!("Array sum: {} (expected: {})", sum, 1024 * 1023 / 2);
        }
        Err(e) => {
            println!("PSRAM allocation failed: {:?}", e);
            println!("Note: PSRAM may not be initialized or available");
        }
    }

    Timer::after(Duration::from_millis(100)).await;
    let stats = rustrtos::mem::psram::stats();
    println!("Updated PSRAM Stats:");
    println!("Used: {} bytes", stats.used);
    println!("Free: {} bytes", stats.free);

    println!("PSRAM demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("PSRAM Demo Example");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    match rustrtos::mem::psram::init() {
        Ok(info) => {
            println!("PSRAM initialized: {} bytes", info.size);
        }
        Err(e) => {
            println!("PSRAM init failed: {:?}", e);
        }
    }

    spawner.spawn(psram_demo_task()).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
