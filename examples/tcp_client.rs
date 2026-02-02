//! TCP 客户端示例
//!
//! 演示如何使用 TCP 客户端连接到服务器并发送 HTTP 请求。
//!
//! # 配置
//! 修改 WIFI_SSID, WIFI_PASSWORD 和目标服务器地址。
//!
//! # 运行
//! ```bash
//! cargo run --example tcp_client --features network,dev --target xtensa-esp32s3-none-elf
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
    const HEAP_SIZE: usize = 72 * 1024; // 72KB for WiFi + TCP
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
use core::str::FromStr;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use static_cell::StaticCell;

use rustrtos::net::wifi::{WifiController, WifiEvent, WifiMode};
use rustrtos::net::tcp::{TcpClient, NetworkStack, StackConfig, Ipv4Address};
use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;

// ===== 配置 =====
const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";

// 目标 HTTP 服务器 (httpbin.org 或本地服务器)
const SERVER_IP: [u8; 4] = [93, 184, 216, 34]; // example.com
const SERVER_PORT: u16 = 80;

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

/// HTTP GET 请求
const HTTP_REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n";

/// TCP 客户端任务
#[embassy_executor::task]
async fn tcp_client_task(
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    connected_signal: &'static Signal<CriticalSectionRawMutex, bool>,
) {
    println!("TCP client task started");
    
    // =========================================
    // 1. WiFi 连接
    // =========================================
    let mut wifi_ctrl = WifiController::new(event_channel, connected_signal);
    
    println!("Initializing WiFi controller...");
    if let Err(e) = wifi_ctrl.init().await {
        println!("WiFi init failed: {:?}", e);
        return;
    }
    
    if let Err(e) = wifi_ctrl.set_mode(WifiMode::Sta).await {
        println!("Set mode failed: {:?}", e);
        return;
    }
    
    println!("Connecting to WiFi '{}'...", WIFI_SSID);
    if let Err(e) = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await {
        println!("WiFi connect failed: {:?}", e);
        return;
    }
    
    println!("Waiting for IP...");
    let local_ip = match wifi_ctrl.wait_for_ip().await {
        Ok(ip) => {
            println!("Got IP: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
            ip
        }
        Err(e) => {
            println!("Failed to get IP: {:?}", e);
            return;
        }
    };
    
    // =========================================
    // 2. 初始化网络栈
    // =========================================
    println!("\nInitializing network stack...");
    let mut stack = NetworkStack::new(StackConfig::default());
    
    if let Err(e) = stack.init().await {
        println!("Stack init failed: {:?}", e);
        return;
    }
    
    if let Err(e) = stack.start_dhcp().await {
        println!("DHCP failed: {:?}", e);
        return;
    }
    
    println!("Network stack ready");
    
    // =========================================
    // 3. TCP 连接
    // =========================================
    let server_ip = Ipv4Address::new(SERVER_IP[0], SERVER_IP[1], SERVER_IP[2], SERVER_IP[3]);
    let server_addr = SocketAddrV4::new(server_ip.to_std(), SERVER_PORT);
    
    println!("\n=========================================");
    println!("Connecting to {}.{}.{}.{}:{}...",
        SERVER_IP[0], SERVER_IP[1], SERVER_IP[2], SERVER_IP[3], SERVER_PORT);
    
    let mut tcp_client = TcpClient::new();
    
    match tcp_client.connect(server_addr).await {
        Ok(_) => {
            println!("TCP connected!");
            println!("Local port: {}", tcp_client.local_port());
        }
        Err(e) => {
            println!("TCP connect failed: {:?}", e);
            return;
        }
    }
    
    // =========================================
    // 4. 发送 HTTP 请求
    // =========================================
    println!("\nSending HTTP request...");
    println!("---");
    // 打印请求 (安全地处理非 UTF8)
    if let Ok(req_str) = core::str::from_utf8(HTTP_REQUEST) {
        for line in req_str.lines() {
            println!("> {}", line);
        }
    }
    println!("---");
    
    match tcp_client.write(HTTP_REQUEST).await {
        Ok(sent) => println!("Sent {} bytes", sent),
        Err(e) => {
            println!("Send failed: {:?}", e);
            return;
        }
    }
    
    // =========================================
    // 5. 接收响应
    // =========================================
    println!("\nWaiting for response...");
    
    let mut rx_buf = [0u8; 1024];
    let mut total_received = 0usize;
    
    // 简单的接收循环 (实际实现需要更复杂的逻辑)
    for _ in 0..10 {
        Timer::after(Duration::from_millis(500)).await;
        
        match tcp_client.read(&mut rx_buf).await {
            Ok(len) if len > 0 => {
                total_received += len;
                
                // 打印接收到的数据 (作为字符串)
                if let Ok(response) = core::str::from_utf8(&rx_buf[..len]) {
                    for line in response.lines().take(10) {
                        println!("< {}", line);
                    }
                    if response.lines().count() > 10 {
                        println!("< ... (truncated)");
                    }
                }
            }
            Ok(_) => {
                // 没有更多数据
                break;
            }
            Err(e) => {
                println!("Read error: {:?}", e);
                break;
            }
        }
    }
    
    println!("\nTotal received: {} bytes", total_received);
    
    // =========================================
    // 6. 关闭连接
    // =========================================
    println!("Closing connection...");
    if let Err(e) = tcp_client.close().await {
        println!("Close error: {:?}", e);
    }
    
    println!("\n=========================================");
    println!("   TCP Client Demo Complete!");
    println!("=========================================");
    
    // 保持任务运行
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
    println!("   RustRTOS TCP Client Example");
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
    
    // 启动 TCP 客户端任务
    spawner.spawn(tcp_client_task(
        event_channel,
        connected_signal,
    )).ok();
    
    // 主循环
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
