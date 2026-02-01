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
    clock::ClockControl,
    gpio::{Io, Level, Output},
    interrupt::{software::SoftwareInterruptControl, Priority},
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
    timer::timg::TimerGroup,
    macros::ram,
};
use esp_rtos::embassy::InterruptExecutor;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};
use static_cell::StaticCell;

// ===== 条件编译日志 =====
#[cfg(feature = "dev")]
use defmt_rtt as _;

#[cfg(feature = "dev")]
use defmt::{info, debug, warn};

#[cfg(not(feature = "dev"))]
macro_rules! info { ($($t:tt)*) => {} }
#[cfg(not(feature = "dev"))]
macro_rules! debug { ($($t:tt)*) => {} }
#[cfg(not(feature = "dev"))]
macro_rules! warn { ($($t:tt)*) => {} }

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

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
#[ram] // IRAM 执行，最小延迟
async fn high_priority_task() {
    info!("High priority task started (P7, IRAM)");
    
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
#[ram]
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
#[esp_rtos::main]
async fn main(low_prio_spawner: Spawner) {
    // 初始化硬件
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    
    info!("Multi-priority example on ESP32-S3 @ {}MHz", clocks.cpu_clock.to_MHz());
    
    // GPIO
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let led = Output::new(io.pins.gpio2, Level::Low);
    
    // 定时器
    let timg0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    
    // 软件中断
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    
    // 启动 esp-rtos
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    info!("esp-rtos started");
    
    // ===== 高优先级执行器 (Priority 7) =====
    let high_prio_executor = InterruptExecutor::new(sw_int.software_interrupt2);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);
    let high_prio_spawner = high_prio_executor.start(Priority::Priority7);
    
    high_prio_spawner.must_spawn(high_priority_task());
    info!("High priority executor started (P7)");
    
    // ===== 中优先级执行器 (Priority 5) =====
    let mid_prio_executor = InterruptExecutor::new(sw_int.software_interrupt1);
    let mid_prio_executor = MID_PRIO_EXECUTOR.init(mid_prio_executor);
    let mid_prio_spawner = mid_prio_executor.start(Priority::Priority5);
    
    mid_prio_spawner.must_spawn(mid_priority_task());
    info!("Mid priority executor started (P5)");
    
    // ===== 低优先级任务 =====
    low_prio_spawner.must_spawn(led_task(led));
    low_prio_spawner.must_spawn(stats_task());
    info!("Low priority tasks spawned");
    
    info!("=== All tasks running ===");
    info!("High (P7): 100μs period - sensor sampling");
    info!("Mid (P5):  10ms period  - data processing");
    info!("Low:       background   - stats & LED");
    
    // 主循环
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
