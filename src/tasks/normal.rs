//! 普通优先级任务
//!
//! 本模块中的任务运行在中/低优先级执行器上，
//! 适用于非实时敏感的操作:
//! - LED 指示
//! - 日志输出
//! - 数据处理
//! - 后台维护

use embassy_time::{Duration, Timer, Ticker};
use embassy_futures::select::{select, Either};
use esp_hal::gpio::Output;

use crate::util::log::*;
use crate::tasks::critical::{get_sensor_value, get_sample_count, wait_sensor_data};
use crate::sync::primitives::CriticalSignal;

// ===== 任务间通信信号 =====
/// LED 控制信号
pub static LED_CONTROL: CriticalSignal<bool> = CriticalSignal::new();

// ===== 中优先级任务: 周期性处理 =====
/// 周期性数据处理任务
///
/// 运行在 Priority2 中断执行器上
/// 每 10ms 处理一次传感器数据
#[embassy_executor::task]
pub async fn periodic_task() {
    log_info!("Periodic task started (Priority2)");
    
    let mut ticker = Ticker::every(Duration::from_millis(10));
    let mut processed_count: u64 = 0;
    
    loop {
        ticker.next().await;
        
        // 读取当前传感器值
        let sensor_value = get_sensor_value();
        
        // 简单数据处理 (移动平均模拟)
        let _processed = process_sensor_data(sensor_value);
        
        processed_count += 1;
        
        // 每 100 次处理输出一次状态
        if processed_count % 100 == 0 {
            let sample_count = get_sample_count();
            log_debug!(
                "Processed {} batches, total samples: {}",
                processed_count,
                sample_count
            );
        }
    }
}

/// 传感器数据处理 (示例: 简单滤波)
#[inline]
fn process_sensor_data(value: u32) -> u32 {
    // 简单的低通滤波模拟
    static mut FILTER_STATE: u32 = 0;
    
    unsafe {
        // alpha = 0.125 (1/8), 使用位移避免浮点
        FILTER_STATE = FILTER_STATE - (FILTER_STATE >> 3) + (value >> 3);
        FILTER_STATE
    }
}

// ===== 低优先级任务: LED 闪烁 =====
/// LED 闪烁任务
///
/// 运行在主执行器 (最低优先级)
/// 支持外部控制 LED 状态
#[embassy_executor::task]
pub async fn led_blink_task(mut led: Output<'static>) {
    log_info!("LED blink task started (low priority)");
    
    let mut led_on = false;
    let blink_interval = Duration::from_millis(500);
    
    loop {
        // 使用 select 同时等待定时器和外部控制信号
        match select(
            Timer::after(blink_interval),
            LED_CONTROL.wait(),
        ).await {
            Either::First(_) => {
                // 定时器触发: 切换 LED 状态
                led_on = !led_on;
                if led_on {
                    led.set_high();
                } else {
                    led.set_low();
                }
            }
            Either::Second(force_on) => {
                // 外部控制: 强制设置 LED 状态
                led_on = force_on;
                if led_on {
                    led.set_high();
                } else {
                    led.set_low();
                }
                log_debug!("LED forced to {}", if led_on { "ON" } else { "OFF" });
            }
        }
    }
}

// ===== 低优先级任务: 后台处理 =====
/// 后台维护任务
///
/// 运行在主执行器，执行非关键的后台操作:
/// - 系统状态监控
/// - 内存统计
/// - 日志聚合
#[embassy_executor::task]
pub async fn background_task() {
    log_info!("Background task started");
    
    let mut iteration: u64 = 0;
    
    loop {
        // 等待传感器批量数据就绪
        let latest_value = wait_sensor_data().await;
        
        iteration += 1;
        
        // 每次收到信号时输出状态
        let total_samples = get_sample_count();
        let samples_per_sec = total_samples / iteration.max(1);
        
        log_info!(
            "Background: iteration={}, latest={}, total_samples={}, rate≈{}/s",
            iteration,
            latest_value,
            total_samples,
            samples_per_sec * 10000  // 因为每10000次采样发一次信号
        );
    }
}

// ===== 任务控制接口 =====

/// 控制 LED 状态
pub fn set_led(on: bool) {
    LED_CONTROL.signal(on);
}

/// 打开 LED
pub fn led_on() {
    set_led(true);
}

/// 关闭 LED
pub fn led_off() {
    set_led(false);
}
