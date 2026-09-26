#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU32, Ordering};

use rustrtos::perf::{cycles, cpu_freq_hz, LatencyStats};
use rustrtos::sync::primitives::CriticalSignal;
use rustrtos::sync::ringbuffer::RingBuffer;
use rustrtos::mem::pool::DramPool;

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

const SAMPLES: usize = 2000;

static SPAWN_STATS: LatencyStats = LatencyStats::new();
static WAKE_STATS: LatencyStats = LatencyStats::new();
static TICK_STATS: LatencyStats = LatencyStats::new();
static ALLOC_STATS: LatencyStats = LatencyStats::new();
static RING_STATS: LatencyStats = LatencyStats::new();

static SPAWN_START: AtomicU32 = AtomicU32::new(0);
static WAKE_START: AtomicU32 = AtomicU32::new(0);
static WAKE_SIGNAL: CriticalSignal<u32> = CriticalSignal::new();

static POOL: DramPool<u32, 32> = DramPool::new();
static RING: RingBuffer<u32, 256> = RingBuffer::new();

#[embassy_executor::task]
async fn spawn_probe() {
    SPAWN_STATS.record(cycles().wrapping_sub(SPAWN_START.load(Ordering::Relaxed)));
}

#[embassy_executor::task]
async fn wake_waiter() {
    loop {
        WAKE_SIGNAL.wait().await;
        WAKE_STATS.record(cycles().wrapping_sub(WAKE_START.load(Ordering::Relaxed)));
    }
}

fn report(label: &str, stats: &LatencyStats, freq_hz: u32) {
    match stats.snapshot() {
        Some(s) => println!(
            "{:<22} n={} min={}ns avg={}ns max={}ns",
            label,
            s.samples,
            s.min_ns(freq_hz),
            s.avg_ns(freq_hz),
            s.max_ns(freq_hz)
        ),
        None => println!("{:<22} no samples", label),
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let freq = cpu_freq_hz();
    println!("RustRTOS latency probe @ {} Hz", freq);
    println!("samples per metric: {}", SAMPLES);

    spawner.spawn(wake_waiter()).ok();

    for _ in 0..SAMPLES {
        SPAWN_START.store(cycles(), Ordering::Relaxed);
        spawner.spawn(spawn_probe()).ok();
        embassy_futures::yield_now().await;
    }

    for i in 0..SAMPLES {
        WAKE_START.store(cycles(), Ordering::Relaxed);
        WAKE_SIGNAL.signal(i as u32);
        embassy_futures::yield_now().await;
    }

    let mut ticker = Ticker::every(Duration::from_millis(1));
    let mut prev = cycles();
    for _ in 0..SAMPLES {
        ticker.next().await;
        let now = cycles();
        TICK_STATS.record(now.wrapping_sub(prev));
        prev = now;
    }

    for i in 0..SAMPLES {
        let sample = rustrtos::perf::ScopedSample::begin(&ALLOC_STATS);
        let slot = POOL.alloc_init(i as u32);
        if let Ok(slot) = slot {
            drop(slot);
        } else {
            sample.cancel();
        }
    }

    for i in 0..SAMPLES {
        let sample = rustrtos::perf::ScopedSample::begin(&RING_STATS);
        if !RING.try_push(i as u32) {
            sample.cancel();
            continue;
        }
        let _ = RING.try_pop();
    }

    println!("--- results ---");
    report("spawn->first poll", &SPAWN_STATS, freq);
    report("signal->resume", &WAKE_STATS, freq);
    report("1ms tick interval", &TICK_STATS, freq);
    report("pool alloc+free", &ALLOC_STATS, freq);
    report("ring push+pop", &RING_STATS, freq);

    loop {
        embassy_time::Timer::after(Duration::from_secs(3600)).await;
    }
}
