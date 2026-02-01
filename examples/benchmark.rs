//! 基准测试示例 - 性能测量
//!
//! 测量 RustRTOS 的关键性能指标:
//! - 中断响应延迟
//! - 任务切换时间
//! - 原子操作开销
//! - 环形缓冲区吞吐量
//!
//! # 运行
//! ```bash
//! cargo run --example benchmark --release --features dev
//! ```

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    clock::ClockControl,
    interrupt::{software::SoftwareInterruptControl, Priority},
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
    timer::timg::TimerGroup,
    macros::ram,
};
use esp_rtos::embassy::InterruptExecutor;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};
use static_cell::StaticCell;
use core::cell::UnsafeCell;

// ===== 日志 =====
#[cfg(feature = "dev")]
use defmt_rtt as _;

#[cfg(feature = "dev")]
use defmt::info;

#[cfg(not(feature = "dev"))]
macro_rules! info { ($($t:tt)*) => {} }

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

// ===== 静态分配 =====
static HIGH_PRIO_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();

// ===== 测试状态 =====
static PING_SIGNAL: Signal<CriticalSectionRawMutex, Instant> = Signal::new();
static PONG_SIGNAL: Signal<CriticalSectionRawMutex, Instant> = Signal::new();

static LATENCY_SUM: AtomicU64 = AtomicU64::new(0);
static LATENCY_COUNT: AtomicU32 = AtomicU32::new(0);
static LATENCY_MIN: AtomicU32 = AtomicU32::new(u32::MAX);
static LATENCY_MAX: AtomicU32 = AtomicU32::new(0);

// ===== 环形缓冲区测试 =====
const RING_SIZE: usize = 1024;

#[repr(C, align(32))]
struct TestRingBuffer {
    buffer: UnsafeCell<[u8; RING_SIZE]>,
    head: AtomicU32,
    tail: AtomicU32,
}

unsafe impl Sync for TestRingBuffer {}

impl TestRingBuffer {
    const fn new() -> Self {
        Self {
            buffer: UnsafeCell::new([0u8; RING_SIZE]),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }
    
    #[inline(always)]
    fn push(&self, value: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        
        if head.wrapping_sub(tail) >= RING_SIZE as u32 {
            return false;
        }
        
        unsafe {
            let ptr = (*self.buffer.get()).as_mut_ptr();
            *ptr.add((head as usize) & (RING_SIZE - 1)) = value;
        }
        
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }
    
    #[inline(always)]
    fn pop(&self) -> Option<u8> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        
        if head == tail {
            return None;
        }
        
        let value = unsafe {
            let ptr = (*self.buffer.get()).as_ptr();
            *ptr.add((tail as usize) & (RING_SIZE - 1))
        };
        
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

static TEST_BUFFER: TestRingBuffer = TestRingBuffer::new();

// ===== 高优先级任务: 延迟测量响应端 =====
#[embassy_executor::task]
#[ram]
async fn high_prio_responder() {
    info!("High priority responder started");
    
    loop {
        // 等待 ping
        let ping_time = PING_SIGNAL.wait().await;
        
        // 立即记录响应时间
        let pong_time = Instant::now();
        
        // 计算延迟
        let latency_us = pong_time.duration_since(ping_time).as_micros() as u32;
        
        // 更新统计
        LATENCY_SUM.fetch_add(latency_us as u64, Ordering::Relaxed);
        LATENCY_COUNT.fetch_add(1, Ordering::Relaxed);
        
        // 更新最小值
        let mut current_min = LATENCY_MIN.load(Ordering::Relaxed);
        while latency_us < current_min {
            match LATENCY_MIN.compare_exchange_weak(
                current_min,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }
        
        // 更新最大值
        let mut current_max = LATENCY_MAX.load(Ordering::Relaxed);
        while latency_us > current_max {
            match LATENCY_MAX.compare_exchange_weak(
                current_max,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
        
        // 发送 pong
        PONG_SIGNAL.signal(pong_time);
    }
}

// ===== 基准测试任务 =====
#[embassy_executor::task]
async fn benchmark_task() {
    info!("Benchmark task started");
    info!("Running benchmarks...");
    
    // 预热
    Timer::after(Duration::from_millis(100)).await;
    
    // ===== 测试 1: 中断响应延迟 =====
    info!("");
    info!("=== Test 1: Interrupt Response Latency ===");
    
    const LATENCY_ITERATIONS: u32 = 10000;
    
    for i in 0..LATENCY_ITERATIONS {
        // 发送 ping
        let ping_time = Instant::now();
        PING_SIGNAL.signal(ping_time);
        
        // 等待 pong
        PONG_SIGNAL.wait().await;
        
        // 短暂延迟避免饱和
        if i % 100 == 0 {
            Timer::after(Duration::from_micros(10)).await;
        }
    }
    
    let count = LATENCY_COUNT.load(Ordering::Relaxed);
    let sum = LATENCY_SUM.load(Ordering::Relaxed);
    let min = LATENCY_MIN.load(Ordering::Relaxed);
    let max = LATENCY_MAX.load(Ordering::Relaxed);
    let avg = if count > 0 { sum / count as u64 } else { 0 };
    
    info!("Latency results ({} samples):", count);
    info!("  Min: {}μs", min);
    info!("  Max: {}μs", max);
    info!("  Avg: {}μs", avg);
    
    // ===== 测试 2: 原子操作吞吐量 =====
    info!("");
    info!("=== Test 2: Atomic Operation Throughput ===");
    
    const ATOMIC_ITERATIONS: u32 = 1_000_000;
    static TEST_ATOMIC: AtomicU32 = AtomicU32::new(0);
    
    let start = Instant::now();
    for _ in 0..ATOMIC_ITERATIONS {
        TEST_ATOMIC.fetch_add(1, Ordering::Relaxed);
    }
    let elapsed_us = start.elapsed().as_micros();
    
    let ops_per_sec = if elapsed_us > 0 {
        (ATOMIC_ITERATIONS as u64 * 1_000_000) / elapsed_us
    } else {
        0
    };
    
    info!("Atomic fetch_add ({} iterations):", ATOMIC_ITERATIONS);
    info!("  Total time: {}μs", elapsed_us);
    info!("  Throughput: {} ops/sec", ops_per_sec);
    info!("  Per op: {}ns", (elapsed_us * 1000) / ATOMIC_ITERATIONS as u64);
    
    // ===== 测试 3: 环形缓冲区吞吐量 =====
    info!("");
    info!("=== Test 3: Ring Buffer Throughput ===");
    
    const RING_ITERATIONS: u32 = 100_000;
    
    // Push 测试
    let start = Instant::now();
    for i in 0..RING_ITERATIONS {
        while !TEST_BUFFER.push((i & 0xFF) as u8) {
            // 缓冲区满，弹出一个
            TEST_BUFFER.pop();
        }
    }
    let push_elapsed = start.elapsed().as_micros();
    
    // 清空
    while TEST_BUFFER.pop().is_some() {}
    
    // Pop 测试
    for i in 0..RING_SIZE as u32 {
        TEST_BUFFER.push((i & 0xFF) as u8);
    }
    
    let start = Instant::now();
    let mut pop_count: u32 = 0;
    while TEST_BUFFER.pop().is_some() {
        pop_count += 1;
    }
    let pop_elapsed = start.elapsed().as_micros();
    
    info!("Ring buffer ({} capacity):", RING_SIZE);
    info!("  Push: {} ops in {}μs", RING_ITERATIONS, push_elapsed);
    info!("  Pop:  {} ops in {}μs", pop_count, pop_elapsed);
    
    // ===== 测试 4: 临界区开销 =====
    info!("");
    info!("=== Test 4: Critical Section Overhead ===");
    
    const CS_ITERATIONS: u32 = 100_000;
    static mut CS_COUNTER: u32 = 0;
    
    let start = Instant::now();
    for _ in 0..CS_ITERATIONS {
        critical_section::with(|_cs| {
            unsafe { CS_COUNTER += 1; }
        });
    }
    let cs_elapsed = start.elapsed().as_micros();
    
    info!("Critical section ({} iterations):", CS_ITERATIONS);
    info!("  Total time: {}μs", cs_elapsed);
    info!("  Per operation: {}ns", (cs_elapsed * 1000) / CS_ITERATIONS as u64);
    
    // ===== 测试 5: Timer 精度 =====
    info!("");
    info!("=== Test 5: Timer Precision ===");
    
    const TIMER_ITERATIONS: u32 = 100;
    let target_us: u64 = 1000; // 1ms
    let mut timer_jitter_sum: u64 = 0;
    let mut timer_max_jitter: u64 = 0;
    
    for _ in 0..TIMER_ITERATIONS {
        let start = Instant::now();
        Timer::after(Duration::from_micros(target_us)).await;
        let elapsed = start.elapsed().as_micros();
        
        let jitter = if elapsed > target_us {
            elapsed - target_us
        } else {
            target_us - elapsed
        };
        
        timer_jitter_sum += jitter;
        if jitter > timer_max_jitter {
            timer_max_jitter = jitter;
        }
    }
    
    let avg_jitter = timer_jitter_sum / TIMER_ITERATIONS as u64;
    
    info!("Timer precision (target: {}μs, {} samples):", target_us, TIMER_ITERATIONS);
    info!("  Avg jitter: {}μs", avg_jitter);
    info!("  Max jitter: {}μs", timer_max_jitter);
    
    // ===== 总结 =====
    info!("");
    info!("========== BENCHMARK COMPLETE ==========");
    info!("System: ESP32-S3 @ 240MHz");
    info!("Interrupt latency (avg): {}μs", avg);
    info!("Atomic ops/sec: {}", ops_per_sec);
    info!("Timer jitter (avg): {}μs", avg_jitter);
    
    if avg < 10 && timer_max_jitter < 100 {
        info!("Result: EXCELLENT - Real-time capable");
    } else if avg < 50 && timer_max_jitter < 500 {
        info!("Result: GOOD - Soft real-time capable");
    } else {
        info!("Result: NEEDS OPTIMIZATION");
    }
    
    // 保持运行
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

// ===== 主入口 =====
#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    
    info!("========================================");
    info!("  RustRTOS Benchmark Suite");
    info!("  ESP32-S3 @ {}MHz", clocks.cpu_clock.to_MHz());
    info!("========================================");
    
    // 定时器
    let timg0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    
    // 软件中断
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    
    // 启动 esp-rtos
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    
    // 高优先级执行器
    let high_prio_executor = InterruptExecutor::new(sw_int.software_interrupt2);
    let high_prio_executor = HIGH_PRIO_EXECUTOR.init(high_prio_executor);
    let high_prio_spawner = high_prio_executor.start(Priority::Priority7);
    
    // 启动高优先级响应任务
    high_prio_spawner.must_spawn(high_prio_responder());
    
    // 启动基准测试任务
    spawner.must_spawn(benchmark_task());
    
    // 主循环
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
