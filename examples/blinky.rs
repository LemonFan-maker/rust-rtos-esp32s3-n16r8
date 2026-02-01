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
    clock::ClockControl,
    gpio::{Io, Level, Output},
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
    timer::timg::TimerGroup,
};

// 条件编译: 开发模式使用 defmt
#[cfg(feature = "dev")]
use defmt_rtt as _;

#[cfg(feature = "dev")]
use defmt::info;

#[cfg(not(feature = "dev"))]
macro_rules! info { ($($t:tt)*) => {} }

// Panic handler
#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

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

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    // 初始化外设
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    
    info!("Blinky example starting on ESP32-S3 @ {}MHz", clocks.cpu_clock.to_MHz());
    
    // 初始化 GPIO
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    
    // 配置 LED 引脚 (GPIO2 是很多开发板的板载 LED)
    let led = Output::new(io.pins.gpio2, Level::Low);
    
    // 初始化定时器
    let timg0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    
    // 初始化 esp-rtos
    let sw_int = esp_hal::interrupt::software::SoftwareInterruptControl::new(
        peripherals.SW_INTERRUPT
    );
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    
    info!("Spawning blink task...");
    
    // 启动 LED 闪烁任务
    spawner.must_spawn(blink_task(led));
    
    // 主循环保持运行
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
