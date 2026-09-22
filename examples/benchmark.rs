#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer, TICK_HZ};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

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

static TASK_SWITCHES: AtomicU32 = AtomicU32::new(0);

const SPAWN_TEST_N: usize = 64;
static SPAWN_DONE: AtomicU32 = AtomicU32::new(0);
static SPAWN_ARRIVALS_NS: [AtomicU64; SPAWN_TEST_N] = [const { AtomicU64::new(0) }; SPAWN_TEST_N];

#[embassy_executor::task(pool_size = SPAWN_TEST_N)]
async fn one_shot_arrival(start: Instant) {
    let idx = SPAWN_DONE.fetch_add(1, Ordering::Relaxed);
    if (idx as usize) < SPAWN_TEST_N {
        SPAWN_ARRIVALS_NS[idx as usize].store(start.elapsed().as_ticks() * (1_000_000_000 / TICK_HZ), Ordering::Relaxed);
    }
}

#[embassy_executor::task]
async fn measure_task_switch() {
    println!("Starting task switch benchmark...");

    let iterations = 10000;
    let start = Instant::now();

    for _ in 0..iterations {
        embassy_futures::yield_now().await;
        TASK_SWITCHES.fetch_add(1, Ordering::Relaxed);
    }

    let elapsed = start.elapsed();
    let ns_per_switch = elapsed.as_micros() * 1000 / iterations as u64;

    println!("Task switch benchmark complete:");
    println!("{} iterations in {} ms", iterations, elapsed.as_millis());
    println!("Average: {} ns per switch", ns_per_switch);
}

#[embassy_executor::task]
async fn measure_timer_precision() {
    println!("Starting timer precision benchmark...");

    let test_delays = [1u64, 10, 100, 1000, 10000];

    for delay_us in test_delays {
        let target = Duration::from_micros(delay_us);
        let start = Instant::now();
        Timer::after(target).await;
        let actual = start.elapsed();

        let error_us = if actual.as_micros() >= delay_us {
            actual.as_micros() - delay_us
        } else {
            delay_us - actual.as_micros()
        };

        println!("Target: {} us, Actual: {} us, Error: {} us",
            delay_us, actual.as_micros(), error_us);
    }

    println!("Timer precision benchmark complete");
}

#[embassy_executor::task]
async fn reporter_task() {
    Timer::after(Duration::from_secs(10)).await;

    println!("Benchmark Summary");
    println!("Total task switches: {}", TASK_SWITCHES.load(Ordering::Relaxed));
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("RustRTOS Benchmark Suite");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let spawn_start = Instant::now();
    for _ in 0..SPAWN_TEST_N {
        spawner.spawn(one_shot_arrival(spawn_start)).ok();
    }
    while SPAWN_DONE.load(Ordering::Relaxed) < SPAWN_TEST_N as u32 {
        embassy_futures::yield_now().await;
    }
    let total_ns: u64 = SPAWN_ARRIVALS_NS.iter().map(|a| a.load(Ordering::Relaxed)).sum();
    let avg_ns = total_ns / SPAWN_TEST_N as u64;
    println!("Task spawn latency: {} ns avg (N={})", avg_ns, SPAWN_TEST_N);

    spawner.spawn(measure_task_switch()).ok();
    spawner.spawn(measure_timer_precision()).ok();
    spawner.spawn(reporter_task()).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
