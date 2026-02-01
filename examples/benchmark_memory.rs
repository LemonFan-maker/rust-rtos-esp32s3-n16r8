//! 内存基准测试示例
//!
//! 测试各种内存分配策略的性能:
//! - DRAM 分配
//! - 内存池分配
//! - 对比分析
//!
//! # 运行
//! ```bash
//! cargo run --example benchmark_memory --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use rustrtos::mem::pool::{MemoryPool, Backend};
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

// 测试数据结构
#[derive(Default, Clone, Copy)]
struct TestBlock {
    data: [u32; 16], // 64 bytes
}

// 静态内存池
static TEST_POOL: MemoryPool<TestBlock, 64, {Backend::Dram as u8}> = MemoryPool::new();
static TOTAL_ALLOCS: AtomicU32 = AtomicU32::new(0);
static TOTAL_FREES: AtomicU32 = AtomicU32::new(0);

/// 内存池基准测试
#[embassy_executor::task]
async fn pool_benchmark_task() {
    println!("Memory Pool Benchmark Started");
    
    // 测试参数
    let iterations = 5000;
    
    // 预热
    for _ in 0..100 {
        if let Ok(block) = TEST_POOL.alloc() {
            drop(block);
        }
    }
    
    // 测量分配时间
    println!("\n=== Allocation Benchmark ===");
    let start = Instant::now();
    
    for _ in 0..iterations {
        if let Ok(mut block) = TEST_POOL.alloc() {
            // 写入一些数据
            block.data[0] = 0xDEADBEEF;
            TOTAL_ALLOCS.fetch_add(1, Ordering::Relaxed);
            drop(block);
            TOTAL_FREES.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_micros() * 1000 / (iterations * 2) as u64; // alloc + free
    
    println!("Iterations: {}", iterations);
    println!("Total time: {} us", elapsed.as_micros());
    println!("Time per alloc+free: {} ns", ns_per_op);
    
    // 测量满载情况
    println!("\n=== Full Pool Benchmark ===");
    
    let mut handles = heapless::Vec::<_, 64>::new();
    
    let start = Instant::now();
    
    // 分配所有
    for _ in 0..64 {
        if let Ok(block) = TEST_POOL.alloc() {
            handles.push(block).ok();
        }
    }
    
    let alloc_time = start.elapsed();
    println!("Allocated 64 blocks in {} us", alloc_time.as_micros());
    
    // 释放所有
    let start = Instant::now();
    handles.clear();
    let free_time = start.elapsed();
    println!("Freed 64 blocks in {} us", free_time.as_micros());
    
    // 随机模式测试
    println!("\n=== Random Pattern Benchmark ===");
    
    let start = Instant::now();
    let mut held = heapless::Vec::<_, 32>::new();
    
    for i in 0..1000 {
        if i % 3 == 0 && held.len() < 32 {
            // 分配
            if let Ok(block) = TEST_POOL.alloc() {
                held.push(block).ok();
            }
        } else if !held.is_empty() {
            // 释放
            held.pop();
        }
    }
    
    let pattern_time = start.elapsed();
    println!("1000 random ops in {} us", pattern_time.as_micros());
    
    // 最终统计
    println!("\n=== Summary ===");
    println!("Total allocations: {}", TOTAL_ALLOCS.load(Ordering::Relaxed));
    println!("Total frees: {}", TOTAL_FREES.load(Ordering::Relaxed));
    println!("Current pool usage: {}/64", TEST_POOL.allocated_count());
    
    println!("\nMemory benchmark complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Memory Benchmark Example");
    println!("========================");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    spawner.spawn(pool_benchmark_task()).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
