//! 高优先级关键任务
//! 本模块中的任务运行在InterruptExecutor (Priority3)上，
//! 具有最高调度优先级，适用于:
//! - 传感器实时采样
//! - 电机控制
//! - 安全关键操作
//! 所有关键函数使用 #[esp_hal::ram]宏放入IRAM，避免Flash访问延迟

use embassy_time::{Duration, Instant, Ticker};
use esp_hal::ram;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use crate::util::log::*;
use crate::sync::primitives::CriticalSignal;

// 共享状态: 传感器数据
/// 最新传感器读数(原子操作，无锁访问)
static SENSOR_VALUE: AtomicU32 = AtomicU32::new(0);

/// 传感器采样计数
static SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);

/// 传感器数据就绪信号
pub static SENSOR_READY: CriticalSignal<u32> = CriticalSignal::new();

// 高优先级任务: 传感器采样
/// 关键传感器采样任务
/// 运行在Priority3中断执行器上，每100μs采样一次
/// 节拍用Ticker按绝对期限对齐: 处理耗时从等待中扣除，采样间隔不随负载漂移
#[embassy_executor::task]
#[ram] // 关键: 放入IRAM避免Flash访问延迟
pub async fn critical_sensor_task() {
    log_info!("Critical sensor task started (Priority3, IRAM)");

    const TARGET_INTERVAL_US: u64 = 100; // 目标采样周期

    let mut ticker = Ticker::every(Duration::from_micros(TARGET_INTERVAL_US));
    let mut last_time = Instant::now();
    let mut max_jitter: u64 = 0;

    loop {
        ticker.next().await;

        // 记录实际采样间隔(用于性能分析)
        let now = Instant::now();
        let elapsed = now.duration_since(last_time).as_micros();
        last_time = now;

        // 计算抖动(jitter): 实际间隔偏离目标周期的幅度
        let jitter = if elapsed > TARGET_INTERVAL_US {
            elapsed - TARGET_INTERVAL_US
        } else {
            TARGET_INTERVAL_US - elapsed
        };

        if jitter > max_jitter {
            max_jitter = jitter;
            log_debug!("New max jitter: {}μs", max_jitter);
        }

        // 模拟传感器读取(实际硬件替换此处)
        let value = simulate_sensor_read();

        // 原子更新传感器值(无锁)
        SENSOR_VALUE.store(value, Ordering::Release);

        // 更新采样计数
        let count = SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);

        // 每10000次采样发送一次信号给低优先级任务
        if (count + 1) % 10000 == 0 {
            SENSOR_READY.signal(value);
        }

        // 超期保护: 落后超过一个周期时，Ticker会背靠背补发错过的节拍，
        // 补发样本的间隔远小于100μs，破坏定频语义; 这里丢弃积压、从当前时刻重新对齐
        if elapsed > 2 * TARGET_INTERVAL_US {
            ticker.reset();
        }
    }
}

/// 模拟传感器读取
/// 实际使用时替换为真实ADC/I2C/SPI读取
#[ram]
fn simulate_sensor_read() -> u32 {
    // 简单的伪随机数生成(LCG)
    static SEED: AtomicU32 = AtomicU32::new(12345);

    let current = SEED.load(Ordering::Relaxed);
    let next = current.wrapping_mul(1103515245).wrapping_add(12345);
    SEED.store(next, Ordering::Relaxed);

    (next >> 16) & 0xFFFF
}

// 公共接口

/// 获取最新传感器值(无锁原子读取)
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
/// 异步等待，不会阻塞其他任务
pub async fn wait_sensor_data() -> u32 {
    SENSOR_READY.wait().await
}

// 性能关键: 中断处理辅助函数

/// 快速位操作 - 放入IRAM(#[ram]要求不可内联，与inline(always)互斥)
#[ram]
pub fn fast_bit_set(value: &mut u32, bit: u8) {
    *value |= 1 << bit;
}

/// 快速位清除 - 放入IRAM(同上)
#[ram]
pub fn fast_bit_clear(value: &mut u32, bit: u8) {
    *value &= !(1 << bit);
}

/// 快速位测试 - 放入IRAM(同上)
#[ram]
pub fn fast_bit_test(value: u32, bit: u8) -> bool {
    (value & (1 << bit)) != 0
}
