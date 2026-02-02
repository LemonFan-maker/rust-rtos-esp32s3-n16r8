//! WiFi 扫描示例 - 使用真实 esp-radio API
//!
//! 演示如何扫描周围的 WiFi 网络。
//!
//! # 运行
//! ```bash
//! cargo run --example wifi_scan --features wifi,dev --release
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
    ModeConfig, WifiController, ClientConfig,
};

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

/// WiFi 扫描任务
#[embassy_executor::task]
async fn wifi_scan_task(wifi_ctrl: &'static mut WifiController<'static>) {
    println!("WiFi scan task started");
    
    // 配置为 Station 模式 (仅用于扫描，不需要密码)
    let station_config = ModeConfig::Client(ClientConfig::default());
    
    if let Err(e) = wifi_ctrl.set_config(&station_config) {
        println!("WiFi set config failed: {:?}", e);
        return;
    }
    
    // 启动 WiFi
    if let Err(e) = wifi_ctrl.start_async().await {
        println!("WiFi start failed: {:?}", e);
        return;
    }
    
    println!("WiFi started successfully");
    
    // 循环扫描
    let mut scan_count = 0u32;
    
    loop {
        scan_count += 1;
        println!("\n========== WiFi Scan #{} ==========", scan_count);
        
        // 执行扫描 (使用默认配置)
        match wifi_ctrl.scan_with_config_async(Default::default()).await {
            Ok(results) => {
                if results.is_empty() {
                    println!("No WiFi networks found");
                } else {
                    println!("Found {} networks:", results.len());
                    println!("{:<32} {:>6} {:>4} {:>8}", "SSID", "RSSI", "CH", "Auth");
                    println!("------------------------------------------------------------");
                    
                    for ap in results {
                        println!(
                            "{:<32} {:>4}dBm {:>4} {:>8?}",
                            ap.ssid.as_str(),
                            ap.signal_strength,
                            ap.channel,
                            ap.auth_method
                        );
                    }
                }
            }
            Err(e) => {
                println!("Scan failed: {:?}", e);
            }
        }
        
        println!("\nNext scan in 10 seconds...");
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();
    
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("=========================================");
    println!("   RustRTOS WiFi Scan Example");
    println!("   ESP32-S3 @ 240MHz");
    println!("=========================================");
    
    // 初始化时钟 (xtensa 平台只需要 timer)
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
    
    // 先存储 radio_controller 到静态区，获取 'static 引用
    static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_ref = RADIO_CONTROLLER.init(radio_controller);
    
    // 创建 WiFi 控制器 (使用 'static 引用)
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
    
    println!("Starting WiFi scan task...\n");
    spawner.spawn(wifi_scan_task(wifi_ctrl)).ok();
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
