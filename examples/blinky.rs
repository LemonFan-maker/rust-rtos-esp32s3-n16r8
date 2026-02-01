//! Blinky 示例 - LED 闪烁
//!
//! 最简单的 RustRTOS 示例，演示:
//! - Embassy 异步任务
//! - GPIO 输出控制
//! - 定时器使用
//!
//! # 运行
//! ```bash
//! cargo run --example blinky --features dev --target xtensa-esp32s3-none-elf
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    timer::timg::TimerGroup,
};

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

/// LED 闪烁任务
#[embassy_executor::task]
async fn blink_task(mut led: Output<'static>) {
    println!("Blink task started");
    
    let mut count: u32 = 0;
    
    loop {
        // LED 开
        led.set_high();
        println!("LED ON (count: {})", count);
        Timer::after(Duration::from_millis(500)).await;
        
        // LED 关
        led.set_low();
        println!("LED OFF");
        Timer::after(Duration::from_millis(500)).await;
        
        count += 1;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("Blinky example starting on ESP32-S3 @ 240MHz");
    
    // 初始化 esp-rtos 时间驱动
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 配置 LED 引脚 (GPIO2 是很多开发板的板载 LED)
    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    
    println!("Spawning blink task...");
    
    // 启动 LED 闪烁任务
    spawner.spawn(blink_task(led)).ok();
    
    // 主循环保持运行
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
