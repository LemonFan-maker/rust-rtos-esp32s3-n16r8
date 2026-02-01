//! 多优先级示例 - 演示混合调度
//!
//! 本示例演示 RustRTOS 的核心特性:
//! - 多优先级 InterruptExecutor
//! - 高/中/低优先级任务协作
//! - 跨优先级通信 (Signal)
//!
//! # 运行
//! ```bash
//! cargo run --example multi_priority --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_hal::{
    interrupt::{software::SoftwareInterruptControl, Priority},
    timer::timg::TimerGroup,
};
use esp_rtos::embassy::InterruptExecutor;
use portable_atomic::{AtomicU32, Ordering};
use static_cell::StaticCell;

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

// ===== 共享状态 =====
static HIGH_PRIO_COUNT: AtomicU32 = AtomicU32::new(0);
static DATA_SIGNAL: Signal<CriticalSectionRawMutex, u32> = Signal::new();
static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

/// 高优先级任务 - 模拟快速传感器采样
#[embassy_executor::task]
async fn high_priority_task() {
    println!("High priority task started");
    
    loop {
        // 模拟传感器采样
        let count = HIGH_PRIO_COUNT.fetch_add(1, Ordering::Relaxed);
        
        // 每 100 次发送信号
        if count % 100 == 0 {
            DATA_SIGNAL.signal(count);
        }
        
        Timer::after(Duration::from_micros(100)).await;
    }
}

/// 中优先级任务 - 数据处理
#[embassy_executor::task]
async fn medium_priority_task() {
    println!("Medium priority task started");
    
    loop {
        // 等待数据信号
        let count = DATA_SIGNAL.wait().await;
        println!("Received data: {}", count);
        
        // 模拟处理
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// 低优先级后台任务
#[embassy_executor::task]
async fn background_task() {
    println!("Background task started");
    
    loop {
        Timer::after(Duration::from_secs(5)).await;
        let count = HIGH_PRIO_COUNT.load(Ordering::Relaxed);
        println!("Background: total samples = {}", count);
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Multi-priority example starting");
    
    // 初始化定时器
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 软件中断
    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    
    // 创建高优先级执行器
    let high_prio_executor = InterruptExecutor::new(sw_ints.software_interrupt2);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);
    let high_prio_spawner = high_prio_executor.start(Priority::Priority3);
    
    // 启动任务
    high_prio_spawner.spawn(high_priority_task()).ok();
    spawner.spawn(medium_priority_task()).ok();
    spawner.spawn(background_task()).ok();
    
    println!("All tasks spawned");
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
