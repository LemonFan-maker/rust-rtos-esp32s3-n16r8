//! WiFi 连接示例 - 使用真实 esp-radio API
//!
//! 演示如何连接到 WiFi 网络并获取 IP 地址。
//!
//! # 配置
//! 修改 WIFI_SSID 和 WIFI_PASSWORD 常量。
//!
//! # 运行
//! ```bash
//! cargo run --example wifi_connect --features wifi,dev --release
//! ```

#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use static_cell::StaticCell;

// 直接使用 esp-radio API
use esp_radio::wifi::{
    ModeConfig, WifiController, ClientConfig, WifiEvent,
};

// ===== WiFi 配置 =====
const WIFI_SSID: &str = "ESP32S3";
const WIFI_PASSWORD: &str = "213213213";

/// 初始化堆分配器
fn init_heap() {
    const HEAP_SIZE: usize = 72 * 1024;
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();
    
    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            HEAP.as_mut_ptr() as *mut u8,
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {};
}

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

/// WiFi 连接任务
#[embassy_executor::task]
async fn wifi_connect_task(wifi_ctrl: &'static mut WifiController<'static>) {
    println!("WiFi connect task started");
    println!("Target SSID: {}", WIFI_SSID);
    
    // 配置为 Station 模式
    let station_config = ModeConfig::Client(
        ClientConfig::default()
            .with_ssid(WIFI_SSID.try_into().unwrap())
            .with_password(WIFI_PASSWORD.try_into().unwrap())
    );
    
    if let Err(e) = wifi_ctrl.set_config(&station_config) {
        println!("WiFi set config failed: {:?}", e);
        return;
    }
    println!("WiFi config set successfully");
    
    // 启动 WiFi
    if let Err(e) = wifi_ctrl.start_async().await {
        println!("WiFi start failed: {:?}", e);
        return;
    }
    println!("WiFi started");
    
    // 连接到 AP
    println!("Connecting to AP...");
    if let Err(e) = wifi_ctrl.connect_async().await {
        println!("WiFi connect failed: {:?}", e);
        return;
    }
    println!("WiFi connected!");
    
    // 等待连接事件
    println!("Waiting for StaConnected event...");
    wifi_ctrl.wait_for_event(WifiEvent::StaConnected).await;
    println!("StaConnected event received!");
    
    // 获取 MAC 地址
    let mac = esp_radio::wifi::sta_mac();
    println!("STA MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
    
    // 获取 RSSI
    match wifi_ctrl.rssi() {
        Ok(rssi) => println!("Signal strength: {} dBm", rssi),
        Err(e) => println!("Failed to get RSSI: {:?}", e),
    }
    
    println!("\n=========================================");
    println!("   WiFi Connected Successfully!");
    println!("=========================================");
    println!("Note: For IP address, you need to run a DHCP client");
    println!("      using embassy-net stack.");
    
    // 保持连接并监控状态
    let mut connected = true;
    loop {
        Timer::after(Duration::from_secs(5)).await;
        
        match wifi_ctrl.is_connected() {
            Ok(is_connected) => {
                if is_connected != connected {
                    connected = is_connected;
                    if connected {
                        println!("[STATUS] Reconnected!");
                    } else {
                        println!("[STATUS] Disconnected!");
                        // 尝试重连
                        println!("[STATUS] Attempting reconnect...");
                        let _ = wifi_ctrl.connect_async().await;
                    }
                }
            }
            Err(e) => println!("[STATUS] Error checking connection: {:?}", e),
        }
        
        // 每30秒显示 RSSI
        if connected {
            if let Ok(rssi) = wifi_ctrl.rssi() {
                println!("[STATUS] RSSI: {} dBm", rssi);
            }
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();
    
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("=========================================");
    println!("   RustRTOS WiFi Connect Example");
    println!("   ESP32-S3 @ 240MHz");
    println!("=========================================");
    
    // 初始化时钟
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    
    // 初始化 esp-radio 控制器
    let radio_controller = match esp_radio::init() {
        Ok(ctrl) => {
            println!("esp-radio initialized successfully");
            ctrl
        }
        Err(e) => {
            println!("esp-radio init failed: {:?}", e);
            loop { core::hint::spin_loop(); }
        }
    };
    
    // 存储 radio controller
    static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_ref = RADIO_CONTROLLER.init(radio_controller);
    
    // 创建 WiFi 控制器
    let (controller, _interfaces) = match esp_radio::wifi::new(
        radio_ref,
        peripherals.WIFI,
        Default::default(),
    ) {
        Ok(ctrl) => {
            println!("WiFi initialized successfully");
            ctrl
        }
        Err(e) => {
            println!("WiFi init failed: {:?}", e);
            loop { core::hint::spin_loop(); }
        }
    };
    
    static WIFI_CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();
    let wifi_ctrl = WIFI_CONTROLLER.init(controller);
    
    println!("Starting WiFi connect task...\n");
    spawner.spawn(wifi_connect_task(wifi_ctrl)).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
