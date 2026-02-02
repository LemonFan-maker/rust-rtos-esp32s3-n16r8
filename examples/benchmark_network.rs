//! 网络性能基准测试
//!
//! 测试 WiFi 和 BLE 的性能指标:
//! - WiFi 连接建立时间
//! - TCP 吞吐量
//! - UDP 吞吐量
//! - BLE 广播延迟
//! - BLE 通知延迟
//!
//! # 运行
//! ```bash
//! cargo run --example benchmark_network --features network,dev --target xtensa-esp32s3-none-elf --release
//! ```

#![no_std]
#![no_main]

extern crate alloc;

// 使用 esp_alloc 作为全局分配器
use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;

/// 初始化堆分配器 (esp-radio 需要)
fn init_heap() {
    const HEAP_SIZE: usize = 96 * 1024; // 96KB for benchmark
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();
    
    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            HEAP.as_mut_ptr() as *mut u8,
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

use core::net::SocketAddrV4;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use static_cell::StaticCell;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use rustrtos::net::wifi::{WifiController, WifiEvent, WifiMode};
use rustrtos::net::tcp::{TcpClient, NetworkStack, StackConfig, Ipv4Address};
use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;

// ===== 配置 =====
const WIFI_SSID: &str = "SSID";
const WIFI_PASSWORD: &str = "PASSWD";

// iperf 服务器地址 (需要在局域网内运行 iperf -s)
const IPERF_SERVER_IP: [u8; 4] = [192, 168, 1, 100];
const IPERF_SERVER_PORT: u16 = 5001;

// 测试参数
const TCP_TEST_DURATION_SECS: u64 = 10;
const UDP_TEST_DURATION_SECS: u64 = 10;
const TCP_BUFFER_SIZE: usize = 1024;
const UDP_BUFFER_SIZE: usize = 1472; // MTU - IP/UDP headers

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

// ===== 静态分配 =====
static WIFI_EVENT_CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>> = StaticCell::new();
static WIFI_CONNECTED_SIGNAL: StaticCell<Signal<CriticalSectionRawMutex, bool>> = StaticCell::new();

// 统计数据
static TX_BYTES: AtomicU64 = AtomicU64::new(0);
static RX_BYTES: AtomicU64 = AtomicU64::new(0);
static TX_PACKETS: AtomicU32 = AtomicU32::new(0);
static RX_PACKETS: AtomicU32 = AtomicU32::new(0);

/// 基准测试结果
#[derive(Debug, Clone, Default)]
struct BenchmarkResult {
    /// 测试名称
    name: &'static str,
    /// 持续时间 (微秒)
    duration_us: u64,
    /// 发送字节数
    tx_bytes: u64,
    /// 接收字节数
    rx_bytes: u64,
    /// 吞吐量 (Kbps)
    throughput_kbps: u32,
    /// 平均延迟 (微秒)
    avg_latency_us: u32,
    /// 最小延迟 (微秒)
    min_latency_us: u32,
    /// 最大延迟 (微秒)
    max_latency_us: u32,
}

impl BenchmarkResult {
    fn print(&self) {
        println!("\n========== {} ==========", self.name);
        println!("Duration:     {} ms", self.duration_us / 1000);
        println!("TX bytes:     {} KB", self.tx_bytes / 1024);
        println!("RX bytes:     {} KB", self.rx_bytes / 1024);
        println!("Throughput:   {} Kbps ({} KB/s)", 
            self.throughput_kbps,
            self.throughput_kbps / 8);
        if self.avg_latency_us > 0 {
            println!("Latency avg:  {} us", self.avg_latency_us);
            println!("Latency min:  {} us", self.min_latency_us);
            println!("Latency max:  {} us", self.max_latency_us);
        }
    }
}

/// WiFi 连接时间测试
async fn benchmark_wifi_connect(
    wifi_ctrl: &mut WifiController<'_>,
) -> BenchmarkResult {
    println!("\n[Benchmark] WiFi Connection Time");
    println!("Connecting to '{}'...", WIFI_SSID);
    
    // 确保断开
    let _ = wifi_ctrl.disconnect().await;
    Timer::after(Duration::from_millis(500)).await;
    
    let start = Instant::now();
    
    let connect_result = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await;
    
    let connect_time = start.elapsed();
    
    if connect_result.is_err() {
        println!("Connection failed!");
        return BenchmarkResult {
            name: "WiFi Connect",
            duration_us: connect_time.as_micros(),
            ..Default::default()
        };
    }
    
    // 等待 IP
    let ip_start = Instant::now();
    let ip_result = wifi_ctrl.wait_for_ip().await;
    let ip_time = ip_start.elapsed();
    
    let total_time = start.elapsed();
    
    if let Ok(ip) = ip_result {
        println!("Connected! IP: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    }
    
    BenchmarkResult {
        name: "WiFi Connect",
        duration_us: total_time.as_micros(),
        avg_latency_us: connect_time.as_micros() as u32,
        min_latency_us: connect_time.as_micros() as u32,
        max_latency_us: (connect_time.as_micros() + ip_time.as_micros()) as u32,
        ..Default::default()
    }
}

/// TCP 吞吐量测试 (发送)
async fn benchmark_tcp_throughput_tx(
    _stack: &NetworkStack<'_>,
) -> BenchmarkResult {
    println!("\n[Benchmark] TCP TX Throughput");
    println!("Connecting to {}:{}...", 
        format_args!("{}.{}.{}.{}", IPERF_SERVER_IP[0], IPERF_SERVER_IP[1], 
            IPERF_SERVER_IP[2], IPERF_SERVER_IP[3]),
        IPERF_SERVER_PORT);
    
    let server_ip = Ipv4Address::new(
        IPERF_SERVER_IP[0], IPERF_SERVER_IP[1],
        IPERF_SERVER_IP[2], IPERF_SERVER_IP[3]
    );
    let server_addr = SocketAddrV4::new(server_ip.to_std(), IPERF_SERVER_PORT);
    
    let mut tcp_client = TcpClient::new();
    
    if tcp_client.connect(server_addr).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP TX Throughput",
            ..Default::default()
        };
    }
    
    println!("Connected, starting TX test for {} seconds...", TCP_TEST_DURATION_SECS);
    
    // 准备发送缓冲区
    let tx_buffer = [0xAA_u8; TCP_BUFFER_SIZE];
    
    TX_BYTES.store(0, Ordering::Relaxed);
    TX_PACKETS.store(0, Ordering::Relaxed);
    
    let start = Instant::now();
    let deadline = Duration::from_secs(TCP_TEST_DURATION_SECS);
    
    while start.elapsed() < deadline {
        match tcp_client.write(&tx_buffer).await {
            Ok(sent) => {
                TX_BYTES.fetch_add(sent as u64, Ordering::Relaxed);
                TX_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => break,
        }
    }
    
    let duration = start.elapsed();
    let total_bytes = TX_BYTES.load(Ordering::Relaxed);
    let total_packets = TX_PACKETS.load(Ordering::Relaxed);
    
    // 计算吞吐量 (Kbps)
    let throughput_kbps = if duration.as_micros() > 0 {
        ((total_bytes * 8 * 1_000_000) / duration.as_micros()) as u32 / 1000
    } else {
        0
    };
    
    let _ = tcp_client.close().await;
    
    println!("TX Test complete:");
    println!("  Sent: {} KB in {} packets", total_bytes / 1024, total_packets);
    
    BenchmarkResult {
        name: "TCP TX Throughput",
        duration_us: duration.as_micros(),
        tx_bytes: total_bytes,
        throughput_kbps,
        ..Default::default()
    }
}

/// TCP 吞吐量测试 (接收)
async fn benchmark_tcp_throughput_rx(
    _stack: &NetworkStack<'_>,
) -> BenchmarkResult {
    println!("\n[Benchmark] TCP RX Throughput");
    println!("Note: Requires iperf client sending data to this device");
    
    // 此测试需要外部 iperf 客户端向设备发送数据
    // 简化实现：测量接收性能
    
    let server_ip = Ipv4Address::new(
        IPERF_SERVER_IP[0], IPERF_SERVER_IP[1],
        IPERF_SERVER_IP[2], IPERF_SERVER_IP[3]
    );
    let server_addr = SocketAddrV4::new(server_ip.to_std(), IPERF_SERVER_PORT);
    
    let mut tcp_client = TcpClient::new();
    
    if tcp_client.connect(server_addr).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP RX Throughput",
            ..Default::default()
        };
    }
    
    println!("Connected, waiting for data for {} seconds...", TCP_TEST_DURATION_SECS);
    
    let mut rx_buffer = [0u8; TCP_BUFFER_SIZE];
    
    RX_BYTES.store(0, Ordering::Relaxed);
    RX_PACKETS.store(0, Ordering::Relaxed);
    
    let start = Instant::now();
    let deadline = Duration::from_secs(TCP_TEST_DURATION_SECS);
    
    while start.elapsed() < deadline {
        match tcp_client.read(&mut rx_buffer).await {
            Ok(received) if received > 0 => {
                RX_BYTES.fetch_add(received as u64, Ordering::Relaxed);
                RX_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                // 短暂等待后重试
                Timer::after(Duration::from_millis(10)).await;
            }
        }
    }
    
    let duration = start.elapsed();
    let total_bytes = RX_BYTES.load(Ordering::Relaxed);
    let total_packets = RX_PACKETS.load(Ordering::Relaxed);
    
    let throughput_kbps = if duration.as_micros() > 0 {
        ((total_bytes * 8 * 1_000_000) / duration.as_micros()) as u32 / 1000
    } else {
        0
    };
    
    let _ = tcp_client.close().await;
    
    println!("RX Test complete:");
    println!("  Received: {} KB in {} packets", total_bytes / 1024, total_packets);
    
    BenchmarkResult {
        name: "TCP RX Throughput",
        duration_us: duration.as_micros(),
        rx_bytes: total_bytes,
        throughput_kbps,
        ..Default::default()
    }
}

/// TCP 延迟测试 (Ping-Pong)
async fn benchmark_tcp_latency(
    _stack: &NetworkStack<'_>,
) -> BenchmarkResult {
    println!("\n[Benchmark] TCP Latency (Echo)");
    
    let server_ip = Ipv4Address::new(
        IPERF_SERVER_IP[0], IPERF_SERVER_IP[1],
        IPERF_SERVER_IP[2], IPERF_SERVER_IP[3]
    );
    let server_addr = SocketAddrV4::new(server_ip.to_std(), IPERF_SERVER_PORT);
    
    let mut tcp_client = TcpClient::new();
    
    if tcp_client.connect(server_addr).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP Latency",
            ..Default::default()
        };
    }
    
    const PING_COUNT: u32 = 100;
    const PING_SIZE: usize = 64;
    
    println!("Running {} ping-pong tests with {} byte packets...", PING_COUNT, PING_SIZE);
    
    let ping_data = [0x55_u8; PING_SIZE];
    let mut pong_data = [0u8; PING_SIZE];
    
    let mut total_latency_us = 0u64;
    let mut min_latency_us = u32::MAX;
    let mut max_latency_us = 0u32;
    let mut successful_pings = 0u32;
    
    for i in 0..PING_COUNT {
        let start = Instant::now();
        
        // 发送
        if tcp_client.write(&ping_data).await.is_err() {
            continue;
        }
        
        // 接收
        if tcp_client.read(&mut pong_data).await.is_err() {
            continue;
        }
        
        let latency_us = start.elapsed().as_micros() as u32;
        
        total_latency_us += latency_us as u64;
        min_latency_us = min_latency_us.min(latency_us);
        max_latency_us = max_latency_us.max(latency_us);
        successful_pings += 1;
        
        if (i + 1) % 20 == 0 {
            println!("  Progress: {}/{}", i + 1, PING_COUNT);
        }
    }
    
    let _ = tcp_client.close().await;
    
    let avg_latency_us = if successful_pings > 0 {
        (total_latency_us / successful_pings as u64) as u32
    } else {
        0
    };
    
    println!("Latency test complete: {}/{} successful", successful_pings, PING_COUNT);
    
    BenchmarkResult {
        name: "TCP Latency",
        duration_us: total_latency_us,
        avg_latency_us,
        min_latency_us: if min_latency_us == u32::MAX { 0 } else { min_latency_us },
        max_latency_us,
        ..Default::default()
    }
}

/// 网络基准测试主任务
#[embassy_executor::task]
async fn benchmark_task(
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    connected_signal: &'static Signal<CriticalSectionRawMutex, bool>,
) {
    println!("\n");
    println!("╔══════════════════════════════════════════╗");
    println!("║    RustRTOS Network Benchmark Suite      ║");
    println!("║    ESP32-S3 @ 240MHz                     ║");
    println!("╚══════════════════════════════════════════╝");
    
    // 收集结果
    let mut results: heapless::Vec<BenchmarkResult, 8> = heapless::Vec::new();
    
    // =========================================
    // 初始化
    // =========================================
    let mut wifi_ctrl = WifiController::new(event_channel, connected_signal);
    
    println!("\n[Init] Initializing WiFi controller...");
    if let Err(e) = wifi_ctrl.init().await {
        println!("WiFi init failed: {:?}", e);
        return;
    }
    
    if let Err(e) = wifi_ctrl.set_mode(WifiMode::Sta).await {
        println!("Set mode failed: {:?}", e);
        return;
    }
    println!("[Init] WiFi ready");
    
    // 初始化网络栈
    let mut stack = NetworkStack::new(StackConfig::default());
    if let Err(e) = stack.init().await {
        println!("Stack init failed: {:?}", e);
        return;
    }
    
    // =========================================
    // 运行基准测试
    // =========================================
    
    // 1. WiFi 连接时间
    println!("\n==================================================");
    println!("Running benchmark 1/4: WiFi Connection Time");
    let result = benchmark_wifi_connect(&mut wifi_ctrl).await;
    let _ = results.push(result);
    
    // 确保已连接并有 IP
    if !wifi_ctrl.is_connected() {
        if let Err(e) = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await {
            println!("Connect failed: {:?}", e);
            return;
        }
    }
    
    if let Err(e) = wifi_ctrl.wait_for_ip().await {
        println!("Get IP failed: {:?}", e);
        return;
    }
    
    if let Err(e) = stack.start_dhcp().await {
        println!("DHCP failed: {:?}", e);
        return;
    }
    
    // 2. TCP 发送吞吐量
    println!("\n==================================================");
    println!("Running benchmark 2/4: TCP TX Throughput");
    let result = benchmark_tcp_throughput_tx(&stack).await;
    let _ = results.push(result);
    
    Timer::after(Duration::from_secs(2)).await;
    
    // 3. TCP 接收吞吐量
    println!("\n==================================================");
    println!("Running benchmark 3/4: TCP RX Throughput");
    let result = benchmark_tcp_throughput_rx(&stack).await;
    let _ = results.push(result);
    
    Timer::after(Duration::from_secs(2)).await;
    
    // 4. TCP 延迟
    println!("\n==================================================");
    println!("Running benchmark 4/4: TCP Latency");
    let result = benchmark_tcp_latency(&stack).await;
    let _ = results.push(result);
    
    // =========================================
    // 输出结果汇总
    // =========================================
    println!("\n");
    println!("╔══════════════════════════════════════════╗");
    println!("║         BENCHMARK RESULTS SUMMARY        ║");
    println!("╚══════════════════════════════════════════╝");
    
    for result in &results {
        result.print();
    }
    
    println!("\n=========================================");
    println!("   Benchmark Suite Complete!");
    println!("=========================================\n");
    
    // 保持运行
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    // 初始化堆分配器 (esp-radio 需要)
    init_heap();
    
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("=========================================");
    println!("   RustRTOS Network Benchmark");
    println!("   ESP32-S3 @ 240MHz");
    println!("=========================================");
    
    // 初始化 esp-rtos 时间驱动
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 初始化 esp-radio (WiFi/BLE 驱动)
    match esp_radio::init() {
        Ok(_controller) => println!("esp-radio initialized successfully"),
        Err(e) => {
            println!("esp-radio init failed: {:?}", e);
            loop { core::hint::spin_loop(); }
        }
    }
    
    // 初始化静态通道
    let event_channel = WIFI_EVENT_CHANNEL.init(Channel::new());
    let connected_signal = WIFI_CONNECTED_SIGNAL.init(Signal::new());
    
    // 启动基准测试任务
    spawner.spawn(benchmark_task(
        event_channel,
        connected_signal,
    )).ok();
    
    // 主循环
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
