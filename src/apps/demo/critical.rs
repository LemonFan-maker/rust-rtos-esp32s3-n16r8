use embassy_time::{Duration, Instant, Ticker};
use esp_hal::ram;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use crate::sync::primitives::CriticalSignal;
use crate::util::log::*;

static SENSOR_VALUE: AtomicU32 = AtomicU32::new(0);

static SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);

pub static SENSOR_CYCLES: AtomicU32 = AtomicU32::new(0);

pub static SENSOR_READY: CriticalSignal<u32> = CriticalSignal::new();

#[embassy_executor::task]
#[ram]
pub async fn critical_sensor_task() {
    log_info!("Critical sensor task started (Priority3, IRAM)");

    const TARGET_INTERVAL_US: u64 = 100;

    let mut ticker = Ticker::every(Duration::from_micros(TARGET_INTERVAL_US));
    let mut last_time = Instant::now();
    let mut max_jitter: u64 = 0;

    loop {
        ticker.next().await;

        SENSOR_CYCLES.fetch_add(1, Ordering::Relaxed);

        let now = Instant::now();
        let elapsed = now.duration_since(last_time).as_micros();
        last_time = now;

        let jitter = if elapsed > TARGET_INTERVAL_US {
            elapsed - TARGET_INTERVAL_US
        } else {
            TARGET_INTERVAL_US - elapsed
        };

        if jitter > max_jitter {
            max_jitter = jitter;
            log_debug!("New max jitter: {} us", max_jitter);
        }

        let value = simulate_sensor_read();

        SENSOR_VALUE.store(value, Ordering::Release);

        let count = SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);

        if (count + 1) % 10000 == 0 {
            SENSOR_READY.signal(value);
        }

        if elapsed > 2 * TARGET_INTERVAL_US {
            ticker.reset();
        }
    }
}

#[ram]
fn simulate_sensor_read() -> u32 {
    static SEED: AtomicU32 = AtomicU32::new(12345);

    let current = SEED.load(Ordering::Relaxed);
    let next = current.wrapping_mul(1103515245).wrapping_add(12345);
    SEED.store(next, Ordering::Relaxed);

    (next >> 16) & 0xFFFF
}

#[inline(always)]
pub fn get_sensor_value() -> u32 {
    SENSOR_VALUE.load(Ordering::Acquire)
}

#[inline(always)]
pub fn get_sample_count() -> u64 {
    SAMPLE_COUNT.load(Ordering::Relaxed)
}

pub async fn wait_sensor_data() -> u32 {
    SENSOR_READY.wait().await
}

#[ram]
pub fn fast_bit_set(value: &mut u32, bit: u8) {
    *value |= 1 << bit;
}

#[ram]
pub fn fast_bit_clear(value: &mut u32, bit: u8) {
    *value &= !(1 << bit);
}

#[ram]
pub fn fast_bit_test(value: u32, bit: u8) -> bool {
    (value & (1 << bit)) != 0
}
