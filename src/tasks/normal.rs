use embassy_time::{Duration, Instant, Timer, Ticker};
use embassy_futures::select::{select, Either};
use esp_hal::gpio::Output;
use portable_atomic::{AtomicU32, Ordering};

use crate::util::log::*;
use crate::tasks::critical::{get_sensor_value, get_sample_count, wait_sensor_data};
use crate::sync::primitives::CriticalSignal;

pub static LED_CONTROL: CriticalSignal<bool> = CriticalSignal::new();

#[embassy_executor::task]
pub async fn periodic_task() {
    log_info!("Periodic task started (Priority2)");

    let mut ticker = Ticker::every(Duration::from_millis(10));
    let mut processed_count: u64 = 0;

    loop {
        ticker.next().await;

        let sensor_value = get_sensor_value();

        let _processed = process_sensor_data(sensor_value);

        processed_count += 1;

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

#[inline]
fn process_sensor_data(value: u32) -> u32 {
    static FILTER_STATE: AtomicU32 = AtomicU32::new(0);

    let prev = FILTER_STATE.load(Ordering::Relaxed);
    let next = prev - (prev >> 3) + (value >> 3);
    FILTER_STATE.store(next, Ordering::Relaxed);
    next
}

#[embassy_executor::task]
pub async fn led_blink_task(mut led: Output<'static>) {
    log_info!("LED blink task started (low priority)");

    let mut led_on = false;
    let blink_interval = Duration::from_millis(500);

    loop {
        match select(
            Timer::after(blink_interval),
            LED_CONTROL.wait(),
        ).await {
            Either::First(_) => {
                led_on = !led_on;
                if led_on {
                    led.set_high();
                } else {
                    led.set_low();
                }
            }
            Either::Second(force_on) => {
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

#[embassy_executor::task]
pub async fn background_task() {
    log_info!("Background task started");

    let mut iteration: u64 = 0;
    let start = Instant::now();

    loop {
        let latest_value = wait_sensor_data().await;

        iteration += 1;

        let total_samples = get_sample_count();
        let elapsed_us = start.elapsed().as_micros().max(1);
        let samples_per_sec = total_samples * 1_000_000 / elapsed_us as u64;

        log_info!(
            "Background: iteration={}, latest={}, total_samples={}, rate~{}/s",
            iteration,
            latest_value,
            total_samples,
            samples_per_sec
        );
    }
}

pub fn set_led(on: bool) {
    LED_CONTROL.signal(on);
}

pub fn led_on() {
    set_led(true);
}

pub fn led_off() {
    set_led(false);
}
