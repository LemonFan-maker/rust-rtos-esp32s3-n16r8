//! 高优先级关键任务
//!
//! 本模块中的任务运行在 InterruptExecutor (Priority 7) 上，
//! 具有最高调度优先级，适用于:
//! - 传感器实时采样
//! - 电机控制
//! - 安全关键操作
//!
//! 所有关键函数使用 `#[ram]` 宏放入 IRAM，避免 Flash 访问延迟

use embassy_time::{Duration, Instant, Timer};
use esp_hal::ram;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use crate::util::log::*;
use crate::sync::primitives::CriticalSignal;

// ===== 共享状态: 传感器数据 =====
/// 最新传感器读数 (原子操作，无锁访问)
static SENSOR_VALUE: AtomicU32 = AtomicU32::new(0);

/// 传感器采样计数
static SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);

/// 传感器数据就绪信号
pub static SENSOR_READY: CriticalSignal<u32> = CriticalSignal::new();

// ===== 高优先级任务: 传感器采样 =====
/// 关键传感器采样任务
///
/// 运行在 Priority 7 中断执行器上，每 100μs 采样一次
/// 目标延迟: < 1μs 响应时间
#[embassy_executor::task]
#[ram] // 关键: 放入 IRAM 避免 Flash 访问延迟
pub async fn critical_sensor_task() {
    log_info!("Critical sensor task started (Priority 7, IRAM)");
    
    let mut last_time = Instant::now();
    let mut max_jitter: u64 = 0;
    
    loop {
        // 记录实际采样间隔 (用于性能分析)
        let now = Instant::now();
        let elapsed = now.duration_since(last_time).as_micros();
        last_time = now;
        
        // 计算抖动 (jitter)
        let target_interval: u64 = 100; // 100μs
        let jitter = if elapsed > target_interval {
            elapsed - target_interval
        } else {
            target_interval - elapsed
        };
        
        if jitter > max_jitter {
            max_jitter = jitter;
            log_debug!("New max jitter: {}μs", max_jitter);
        }
        
        // 模拟传感器读取 (实际硬件替换此处)
        let value = simulate_sensor_read();
        
        // 原子更新传感器值 (无锁)
        SENSOR_VALUE.store(value, Ordering::Release);
        
        // 更新采样计数
        let count = SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);
        
        // 每 10000 次采样发送一次信号给低优先级任务
        if count % 10000 == 0 {
            SENSOR_READY.signal(value);
        }
        
        // 高精度延时: 100μs
        Timer::after(Duration::from_micros(100)).await;
    }
}

/// 模拟传感器读取
///
/// 实际使用时替换为真实 ADC/I2C/SPI 读取
#[inline(always)]
#[ram]
fn simulate_sensor_read() -> u32 {
    // 简单的伪随机数生成 (LCG)
    static SEED: AtomicU32 = AtomicU32::new(12345);
    
    let current = SEED.load(Ordering::Relaxed);
    let next = current.wrapping_mul(1103515245).wrapping_add(12345);
    SEED.store(next, Ordering::Relaxed);
    
    (next >> 16) & 0xFFFF
}

// ===== 公共接口 =====

/// 获取最新传感器值 (无锁原子读取)
#[inline(always)]
pub fn get_sensor_value() -> u32 {
    SENSOR_VALUE.load(Ordering::Acquire)
}

/// 获取总采样次数
#[inline(always)]
pub fn get_sample_count() -> u64 {
    SAMPLE_COUNT.load(Ordering::Relaxed)
}

/// 等待新的传感器数据
///
/// 异步等待，不会阻塞其他任务
pub async fn wait_sensor_data() -> u32 {
    SENSOR_READY.wait().await
}

// ===== 性能关键: 中断处理辅助函数 =====

/// 快速位操作 - 强制内联
#[inline(always)]
#[ram]
pub fn fast_bit_set(value: &mut u32, bit: u8) {
    *value |= 1 << bit;
}

/// 快速位清除 - 强制内联
#[inline(always)]
#[ram]
pub fn fast_bit_clear(value: &mut u32, bit: u8) {
    *value &= !(1 << bit);
}

/// 快速位测试 - 强制内联
#[inline(always)]
#[ram]
pub fn fast_bit_test(value: u32, bit: u8) -> bool {
    (value & (1 << bit)) != 0
}
