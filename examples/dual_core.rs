//! 双核示例 - SMP 支持演示
//!
//! 演示 ESP32-S3 双核功能:
//! - Core1 启动
//! - 跨核通信
//! - IPC 原语使用
//!
//! # 运行
//! ```bash
//! cargo run --example dual_core --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
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

// ===== 共享计数器 =====
static CORE0_COUNTER: AtomicU32 = AtomicU32::new(0);
static CORE1_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Core0 工作任务
#[embassy_executor::task]
async fn core0_task() {
    println!("Core0 task started");
    
    loop {
        CORE0_COUNTER.fetch_add(1, Ordering::Relaxed);
        Timer::after(Duration::from_millis(100)).await;
    }
}

/// 监控任务
#[embassy_executor::task]
async fn monitor_task() {
    println!("Monitor task started");
    
    loop {
        Timer::after(Duration::from_secs(2)).await;
        
        let c0 = CORE0_COUNTER.load(Ordering::Relaxed);
        let c1 = CORE1_COUNTER.load(Ordering::Relaxed);
        
        println!("=== Core Status ===");
        println!("  Core0 counter: {}", c0);
        println!("  Core1 counter: {} (simulated)", c1);
        println!("  Total: {}", c0 + c1);
    }
}

/// IPC 演示任务
#[embassy_executor::task]
async fn ipc_demo_task() {
    println!("IPC demo task started");
    
    // 使用 IPC 通道进行跨核通信演示
    use rustrtos::tasks::multicore::{IpcChannel, IpcSignal};
    
    static IPC_CHANNEL: IpcChannel<u32, 8> = IpcChannel::new();
    static IPC_SIGNAL: IpcSignal = IpcSignal::new();
    
    // 发送数据
    for i in 0..5 {
        if IPC_CHANNEL.try_send(i).is_ok() {
            println!("Sent {} to IPC channel", i);
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    
    // 接收数据
    println!("\nReceiving from IPC channel:");
    while let Some(value) = IPC_CHANNEL.try_recv() {
        println!("  Received: {}", value);
    }
    
    // 测试信号
    println!("\nTesting IPC signal:");
    IPC_SIGNAL.signal();
    println!("  Signal sent");
    
    if IPC_SIGNAL.check_and_clear() {
        println!("  Signal received and cleared");
    }
    
    println!("\nIPC demo complete!");
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Dual Core Example");
    println!("=================");
    println!("Note: Full dual-core requires hardware support");
    
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 启动任务
    spawner.spawn(core0_task()).ok();
    spawner.spawn(monitor_task()).ok();
    spawner.spawn(ipc_demo_task()).ok();
    
    // 模拟 Core1 活动
    loop {
        CORE1_COUNTER.fetch_add(1, Ordering::Relaxed);
        Timer::after(Duration::from_millis(200)).await;
    }
}
