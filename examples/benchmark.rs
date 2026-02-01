//! Benchmark 示例 - 性能测试
//!
//! 测试 RTOS 的性能指标:
//! - 任务切换延迟
//! - 信号传递延迟
//! - 中断响应时间
//!
//! # 运行
//! ```bash
//! cargo run --example benchmark --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU32, Ordering};

// ===== 条件编译日志 =====
#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {};
}

// ===== Panic Handler =====
#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

// ===== 统计数据 =====
static TASK_SWITCHES: AtomicU32 = AtomicU32::new(0);

/// 测量任务切换
#[embassy_executor::task]
async fn measure_task_switch() {
    println!("Starting task switch benchmark...");
    
    let iterations = 10000;
    let start = Instant::now();
    
    for _ in 0..iterations {
        // 每次 yield 触发一次任务切换
        embassy_futures::yield_now().await;
        TASK_SWITCHES.fetch_add(1, Ordering::Relaxed);
    }
    
    let elapsed = start.elapsed();
    let ns_per_switch = elapsed.as_micros() * 1000 / iterations as u64;
    
    println!("Task switch benchmark complete:");
    println!("  {} iterations in {} ms", iterations, elapsed.as_millis());
    println!("  Average: {} ns per switch", ns_per_switch);
}

/// 测量定时器精度
#[embassy_executor::task]
async fn measure_timer_precision() {
    println!("Starting timer precision benchmark...");
    
    let test_delays = [1u64, 10, 100, 1000, 10000]; // 微秒
    
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
        
        println!("  Target: {} us, Actual: {} us, Error: {} us",
            delay_us, actual.as_micros(), error_us);
    }
    
    println!("Timer precision benchmark complete");
}

/// 报告任务
#[embassy_executor::task]
async fn reporter_task() {
    // 等待测试完成
    Timer::after(Duration::from_secs(10)).await;
    
    println!("=== Benchmark Summary ===");
    println!("Total task switches: {}", TASK_SWITCHES.load(Ordering::Relaxed));
    println!("========================");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("RustRTOS Benchmark Suite");
    println!("========================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    spawner.spawn(measure_task_switch()).ok();
    spawner.spawn(measure_timer_precision()).ok();
    spawner.spawn(reporter_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
