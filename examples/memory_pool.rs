#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::pool::{MemoryPool, Backend};

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

#[derive(Default, Clone)]
struct SensorData {
    timestamp: u32,
    value: i32,
    flags: u8,
}

#[embassy_executor::task]
async fn pool_test_task() {
    println!("Memory Pool Test Started");

    static POOL: MemoryPool<SensorData, 32, {Backend::Dram as u8}> = MemoryPool::new();

    println!("Allocation Performance Test");

    let start = Instant::now();
    let iterations = 1000;
    let mut alloc_count = 0;

    for i in 0..iterations {
        if let Ok(mut item) = POOL.alloc() {
            item.timestamp = i;
            item.value = (i * 2) as i32;
            item.flags = 1;
            alloc_count += 1;
            drop(item);
        }
    }

    let elapsed = start.elapsed();
    println!("Performed {} alloc/free cycles", iterations);
    println!("Total time: {} us", elapsed.as_micros());
    println!("Average: {} ns per cycle", elapsed.as_micros() * 1000 / iterations as u64);

    println!("Concurrent Usage Test");

    let mut items = heapless::Vec::<_, 32>::new();
    for i in 0..32 {
        if let Ok(mut item) = POOL.alloc() {
            item.timestamp = i as u32;
            item.value = i as i32;
            items.push(item).ok();
        }
    }

    println!("Allocated {} items", items.len());
    println!("Pool usage: {}/32", POOL.allocated_count());

    let extra = POOL.alloc();
    println!("Extra allocation: {}", if extra.is_ok() { "success" } else { "failed (expected)" });

    for _ in 0..16 {
        items.pop();
    }

    println!("After releasing 16: {}/32 used", POOL.allocated_count());

    for _ in 0..8 {
        if let Ok(item) = POOL.alloc() {
            items.push(item).ok();
        }
    }

    println!("After reallocating 8: {}/32 used", POOL.allocated_count());

    println!("Memory pool test complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Memory Pool Example");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner.spawn(pool_test_task()).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
