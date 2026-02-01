//! Blinky 示例 - LED 闪烁
//!
//! 最简单的 RustRTOS 示例，演示:
//! - Embassy 异步任务
//! - GPIO 输出控制
//! - 定时器使用
//!
//! # 运行
//! ```bash
//! cargo run --example blinky --features dev
//! ```

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output},
    timer::timg::TimerGroup,
};

// 日志输出
use esp_println::println as info;

// defmt 支持（用于 embassy）
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
    project_name: *b"blinky\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
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

/// LED 闪烁任务
#[embassy_executor::task]
async fn blink_task(mut led: Output<'static>) {
    info!("Blink task started");
    
    let mut count: u32 = 0;
    
    loop {
        // LED 开
        led.set_high();
        info!("LED ON (count: {})", count);
        Timer::after(Duration::from_millis(500)).await;
        
        // LED 关
        led.set_low();
        info!("LED OFF");
        Timer::after(Duration::from_millis(500)).await;
        
        count += 1;
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    info!("Blinky example starting on ESP32-S3 @ 240MHz");
    
    // 初始化 Embassy 时间驱动
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_hal_embassy::init(timg0.timer0);
    
    // 配置 LED 引脚 (GPIO2 是很多开发板的板载 LED)
    let led = Output::new(peripherals.GPIO2, Level::Low);
    
    info!("Spawning blink task...");
    
    // 启动 LED 闪烁任务
    spawner.spawn(blink_task(led)).ok();
    
    // 主循环保持运行
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
