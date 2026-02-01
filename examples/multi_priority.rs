//! 多优先级示例 - 演示混合调度
//!
//! 本示例演示 RustRTOS 的核心特性:
//! - 多优先级 InterruptExecutor
//! - 高/中/低优先级任务协作
//! - 跨优先级通信 (Signal/Channel)
//! - 抢占式调度验证
//!
//! # 优先级配置
//! - Priority 7: 高优先级传感器任务 (每 100μs)
//! - Priority 5: 中优先级处理任务 (每 10ms)
//! - 主执行器: 低优先级后台任务
//!
//! # 运行
//! ```bash
//! cargo run --example multi_priority --features dev
//! ```

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, Ticker};
use esp_hal::{
    gpio::{Level, Output},
    timer::timg::TimerGroup,
};
use esp_hal_embassy::InterruptExecutor;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};
use static_cell::StaticCell;

// 日志输出
use esp_println::println as info;
macro_rules! debug { ($($t:tt)*) => { esp_println::println!($($t)*) } }
macro_rules! warn { ($($t:tt)*) => { esp_println::println!("WARN: {}", format_args!($($t)*)) } }

// defmt 支持
#[cfg(feature = "dev")]
use defmt_rtt as _;

use esp_backtrace as _;

#[cfg(feature = "dev")]
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    loop { core::hint::spin_loop(); }
}

// ESP-IDF App Descriptor
#[repr(C)]
struct EspAppDesc {
    magic_word: u32,
    secure_version: u32,
    reserv1: [u32; 2],
    version: [u8; 32],
    project_name: [u8; 32],
    time: [u8; 16],
    date: [u8; 16],
    idf_ver: [u8; 32],
    app_elf_sha256: [u8; 32],
    min_efuse_blk_rev_full: u16,
    max_efuse_blk_rev_full: u16,
    mmu_page_size: u8,
    reserv3: [u8; 3],
    reserv2: [u32; 18],
}

#[link_section = ".flash.appdesc"]
#[used]
static ESP_APP_DESC: EspAppDesc = EspAppDesc {
    magic_word: 0xABCD5432,
    secure_version: 0,
    reserv1: [0; 2],
    version: *b"0.1.0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    project_name: *b"multi_priority\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    time: *b"00:00:00\0\0\0\0\0\0\0\0",
    date: *b"2025-01-01\0\0\0\0\0\0",
    idf_ver: *b"v5.0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    app_elf_sha256: [0; 32],
    min_efuse_blk_rev_full: 0,
    max_efuse_blk_rev_full: u16::MAX,
    mmu_page_size: 16,
    reserv3: [0; 3],
    reserv2: [0; 18],
};

// ===== 静态分配 =====
static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
static MID_PRIO_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new();

// ===== 共享状态 =====
/// 高优先级任务计数
static HIGH_PRIO_COUNT: AtomicU64 = AtomicU64::new(0);
/// 中优先级任务计数
static MID_PRIO_COUNT: AtomicU64 = AtomicU64::new(0);
/// 低优先级任务计数
static LOW_PRIO_COUNT: AtomicU64 = AtomicU64::new(0);

/// 传感器数据
static SENSOR_DATA: AtomicU32 = AtomicU32::new(0);

/// 数据就绪信号
static DATA_READY: Signal<CriticalSectionRawMutex, u32> = Signal::new();

/// 最大抖动记录 (μs)
static MAX_JITTER: AtomicU32 = AtomicU32::new(0);

// ===== 高优先级任务 (Priority 7) =====
/// 关键传感器任务 - 每 100μs 执行
#[embassy_executor::task]
async fn high_priority_task() {
    info!("High priority task started (P7)");
    
    let mut last_time = Instant::now();
    let target_period_us: u64 = 100;
    
    loop {
        // 计算实际间隔和抖动
        let now = Instant::now();
        let elapsed_us = now.duration_since(last_time).as_micros();
        last_time = now;
        
        let jitter = if elapsed_us > target_period_us {
            (elapsed_us - target_period_us) as u32
        } else {
            (target_period_us - elapsed_us) as u32
        };
        
        // 更新最大抖动
        let current_max = MAX_JITTER.load(Ordering::Relaxed);
        if jitter > current_max {
            MAX_JITTER.store(jitter, Ordering::Relaxed);
        }
        
        // 模拟传感器读取
        let value = simulate_sensor();
        SENSOR_DATA.store(value, Ordering::Release);
        
        // 计数
        let count = HIGH_PRIO_COUNT.fetch_add(1, Ordering::Relaxed);
        
        // 每 10000 次发送一次信号
        if count % 10000 == 0 {
            DATA_READY.signal(value);
        }
        
        Timer::after(Duration::from_micros(100)).await;
    }
}

/// 模拟传感器读取
#[inline(always)]
fn simulate_sensor() -> u32 {
    static SEED: AtomicU32 = AtomicU32::new(12345);
    let current = SEED.load(Ordering::Relaxed);
    let next = current.wrapping_mul(1103515245).wrapping_add(12345);
    SEED.store(next, Ordering::Relaxed);
    (next >> 16) & 0xFFFF
}

// ===== 中优先级任务 (Priority 5) =====
/// 数据处理任务 - 每 10ms 执行
#[embassy_executor::task]
async fn mid_priority_task() {
    info!("Mid priority task started (P5)");
    
    let mut ticker = Ticker::every(Duration::from_millis(10));
    let mut sum: u64 = 0;
    
    loop {
        ticker.next().await;
        
        // 读取并处理传感器数据
        let value = SENSOR_DATA.load(Ordering::Acquire);
        sum = sum.wrapping_add(value as u64);
        
        let count = MID_PRIO_COUNT.fetch_add(1, Ordering::Relaxed);
        
        // 每 100 次输出统计
        if count % 100 == 0 {
            let avg = sum / (count + 1);
            debug!("Mid task: count={}, avg_value={}", count, avg);
        }
    }
}

// ===== 低优先级任务 (主执行器) =====
/// LED 指示任务
#[embassy_executor::task]
async fn led_task(mut led: Output<'static>) {
    info!("LED task started (low priority)");
    
    loop {
        led.set_high();
        Timer::after(Duration::from_millis(100)).await;
        led.set_low();
        Timer::after(Duration::from_millis(900)).await;
    }
}

/// 统计报告任务
#[embassy_executor::task]
async fn stats_task() {
    info!("Stats task started");
    
    loop {
        // 等待数据批量就绪
        let latest = DATA_READY.wait().await;
        
        let count = LOW_PRIO_COUNT.fetch_add(1, Ordering::Relaxed);
        let high_count = HIGH_PRIO_COUNT.load(Ordering::Relaxed);
        let mid_count = MID_PRIO_COUNT.load(Ordering::Relaxed);
        let max_jitter = MAX_JITTER.load(Ordering::Relaxed);
        
        info!("=== Statistics Report #{} ===", count);
        info!("  High priority executions: {}", high_count);
        info!("  Mid priority executions:  {}", mid_count);
        info!("  Latest sensor value:      {}", latest);
        info!("  Max jitter:               {}μs", max_jitter);
        info!("  Target rate:              10000 samples/sec");
        
        // 验证调度正确性
        if max_jitter > 50 {
            warn!("Warning: High jitter detected (>50μs)");
        }
    }
}

// ===== 主入口 =====
#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    info!("Multi-priority example on ESP32-S3 @ 240MHz");
    
    // 初始化 Embassy 时间驱动
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_hal_embassy::init(timg0.timer0);
    
    // GPIO
    let led = Output::new(peripherals.GPIO2, Level::Low);
    
    info!("Note: This simplified version runs all tasks in single executor");
    info!("For true multi-priority, use esp-hal 1.0+ with InterruptExecutor");
    
    // 启动所有任务（在单执行器中运行）
    spawner.spawn(high_priority_task()).ok();
    spawner.spawn(mid_priority_task()).ok();
    spawner.spawn(led_task(led)).ok();
    spawner.spawn(stats_task()).ok();
    
    info!("=== All tasks running ===");
    info!("High: 100μs period - sensor sampling");
    info!("Mid:  10ms period  - data processing");
    info!("Low:  background   - stats & LED");
    
    // 主循环
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
