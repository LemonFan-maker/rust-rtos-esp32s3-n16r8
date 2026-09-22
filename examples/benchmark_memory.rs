#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::pool::{MemoryPool, Backend};
use portable_atomic::{AtomicU32, Ordering};

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

#[derive(Default, Clone, Copy)]
struct TestBlock {
    data: [u32; 16],
}

static TEST_POOL: MemoryPool<TestBlock, 64, {Backend::Dram as u8}> = MemoryPool::new();
static TOTAL_ALLOCS: AtomicU32 = AtomicU32::new(0);
static TOTAL_FREES: AtomicU32 = AtomicU32::new(0);

#[embassy_executor::task]
async fn pool_benchmark_task() {
    println!("Memory Pool Benchmark Started");

    let iterations = 5000;

    for _ in 0..100 {
        if let Ok(block) = TEST_POOL.alloc() {
            drop(block);
        }
    }

    println!("Allocation Benchmark");
    let start = Instant::now();

    for _ in 0..iterations {
        if let Ok(mut block) = TEST_POOL.alloc() {
            block.data[0] = 0xDEADBEEF;
            TOTAL_ALLOCS.fetch_add(1, Ordering::Relaxed);
            drop(block);
            TOTAL_FREES.fetch_add(1, Ordering::Relaxed);
        }
    }

    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_micros() * 1000 / (iterations * 2) as u64;

    println!("Iterations: {}", iterations);
    println!("Total time: {} us", elapsed.as_micros());
    println!("Time per alloc+free: {} ns", ns_per_op);

    println!("Full Pool Benchmark");

    let mut handles = heapless::Vec::<_, 64>::new();

    let start = Instant::now();

    const EXHAUST_ATTEMPTS: usize = 72;
    let mut alloc_ok: u32 = 0;
    let mut alloc_fail: u32 = 0;
    for _ in 0..EXHAUST_ATTEMPTS {
        match TEST_POOL.alloc() {
            Ok(block) => {
                alloc_ok += 1;
                handles.push(block).ok();
            }
            Err(_) => {
                alloc_fail += 1;
            }
        }
    }

    let alloc_time = start.elapsed();
    println!("Alloc attempts: {}, failures: {}", EXHAUST_ATTEMPTS, alloc_fail);
    println!("Allocated {} blocks in {} us", alloc_ok, alloc_time.as_micros());

    let start = Instant::now();
    handles.clear();
    let free_time = start.elapsed();
    println!("Freed 64 blocks in {} us", free_time.as_micros());

    println!("Random Pattern Benchmark");

    let start = Instant::now();
    let mut held = heapless::Vec::<_, 32>::new();

    for i in 0..1000 {
        if i % 3 == 0 && held.len() < 32 {
            if let Ok(block) = TEST_POOL.alloc() {
                held.push(block).ok();
            }
        } else if !held.is_empty() {
            held.pop();
        }
    }

    let pattern_time = start.elapsed();
    println!("1000 random ops in {} us", pattern_time.as_micros());

    println!("Summary");
    println!("Total allocations: {}", TOTAL_ALLOCS.load(Ordering::Relaxed));
    println!("Total frees: {}", TOTAL_FREES.load(Ordering::Relaxed));
    println!("Current pool usage: {}/64", TEST_POOL.allocated_count());

    println!("Memory benchmark complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Memory Benchmark Example");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    spawner.spawn(pool_benchmark_task()).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
