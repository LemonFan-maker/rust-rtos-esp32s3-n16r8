//! 内存池示例 - 固定大小内存分配
//!
//! 演示内存池的使用:
//! - 零碎片内存分配
//! - 快速分配/释放
//! - 适用于实时系统
//!
//! # 运行
//! ```bash
//! cargo run --example memory_pool --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::pool::{MemoryPool, Backend};

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

/// 示例数据结构
#[derive(Default, Clone)]
struct SensorData {
    timestamp: u32,
    value: i32,
    flags: u8,
}

/// 内存池测试任务
#[embassy_executor::task]
async fn pool_test_task() {
    println!("Memory Pool Test Started");
    
    // 创建一个包含 32 个 SensorData 的内存池
    static POOL: MemoryPool<SensorData, 32, {Backend::Dram as u8}> = MemoryPool::new();
    
    // 测量分配性能
    println!("\n=== Allocation Performance Test ===");
    
    let start = Instant::now();
    let iterations = 1000;
    let mut alloc_count = 0;
    
    for i in 0..iterations {
        if let Ok(mut item) = POOL.alloc() {
            item.timestamp = i;
            item.value = (i * 2) as i32;
            item.flags = 1;
            alloc_count += 1;
            // 立即释放以便重复使用
            drop(item);
        }
    }
    
    let elapsed = start.elapsed();
    println!("Performed {} alloc/free cycles", iterations);
    println!("Total time: {} us", elapsed.as_micros());
    println!("Average: {} ns per cycle", elapsed.as_micros() * 1000 / iterations as u64);
    
    // 测试并发使用
    println!("\n=== Concurrent Usage Test ===");
    
    // 分配所有槽位
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
    
    // 尝试再分配应该失败
    let extra = POOL.alloc();
    println!("Extra allocation: {}", if extra.is_ok() { "success" } else { "failed (expected)" });
    
    // 释放一半
    for _ in 0..16 {
        items.pop();
    }
    
    println!("After releasing 16: {}/32 used", POOL.allocated_count());
    
    // 再次分配
    for _ in 0..8 {
        if let Ok(item) = POOL.alloc() {
            items.push(item).ok();
        }
    }
    
    println!("After reallocating 8: {}/32 used", POOL.allocated_count());
    
    println!("\nMemory pool test complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Memory Pool Example");
    println!("===================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    spawner.spawn(pool_test_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
