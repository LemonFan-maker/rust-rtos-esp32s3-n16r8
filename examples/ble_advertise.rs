//! BLE 广播示例 - 使用真实 trouble-host API
//!
//! 演示如何使用 trouble-host 进行蓝牙低功耗广播。
//!
//! # 运行
//! ```bash
//! cargo run --example ble_advertise --features ble,dev --release
//! ```

#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::Timer;
use esp_hal::timer::timg::TimerGroup;
use static_cell::StaticCell;

// esp-radio BLE 控制器
use esp_radio::ble::controller::BleConnector;

// trouble-host BLE 协议栈 - 使用 re-export 的 embassy_time
use trouble_host::prelude::*;

// ===== 配置 =====
const DEVICE_NAME: &[u8] = b"RustRTOS-BLE";

// ===== 堆初始化 =====
fn init_heap() {
    const HEAP_SIZE: usize = 72 * 1024; // 72KB for BLE
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();
    
    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            HEAP.as_mut_ptr() as *mut u8,
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

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

// Host 资源配置
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

/// BLE 广播任务
async fn ble_advertise<C: Controller>(controller: C) {
    // 使用随机地址
    let address: Address = Address::random([0x41, 0x5A, 0xE3, 0x1E, 0x83, 0xE7]);
    println!("BLE Address: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", 
        0x41, 0x5A, 0xE3, 0x1E, 0x83, 0xE7);

    // 创建 Host 资源
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = 
        HostResources::new();
    
    // 构建 BLE 协议栈
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address);
    
    let Host {
        mut peripheral,
        mut runner,
        ..
    } = stack.build();

    // 构建广播数据
    let mut adv_data = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(DEVICE_NAME),
        ],
        &mut adv_data[..],
    ).unwrap();

    println!("\n=========================================");
    println!("   BLE Advertising Active");
    println!("   Device Name: {}", core::str::from_utf8(DEVICE_NAME).unwrap_or("RustRTOS"));
    println!("=========================================\n");

    // 运行 BLE 协议栈和广播
    let _ = join(
        runner.run(),
        async {
            loop {
                println!("[BLE] Starting advertising...");
                
                match peripheral.advertise(
                    &Default::default(),
                    Advertisement::ConnectableScannableUndirected {
                        adv_data: &adv_data[..len],
                        scan_data: &[],
                    },
                ).await {
                    Ok(advertiser) => {
                        println!("[BLE] Advertising started, waiting for connection...");
                        
                        // 等待连接
                        match advertiser.accept().await {
                            Ok(conn) => {
                                println!("[BLE] Connection established!");
                                println!("[BLE] Peer: {:?}", conn.peer_address());
                                
                                // 保持连接直到断开
                                while conn.is_connected() {
                                    Timer::after(embassy_time::Duration::from_secs(1)).await;
                                }
                                println!("[BLE] Connection closed");
                            }
                            Err(e) => {
                                println!("[BLE] Accept error: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("[BLE] Advertising error: {:?}", e);
                        Timer::after(embassy_time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    ).await;
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    init_heap();
    
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("=========================================");
    println!("   RustRTOS BLE Advertise Example");
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
    
    // 存储 radio_controller 到静态区
    static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_ref = RADIO_CONTROLLER.init(radio_controller);
    
    // 创建 BLE 控制器
    let connector = BleConnector::new(radio_ref, peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    
    println!("BLE controller initialized");

    // 运行 BLE 广播
    ble_advertise(controller).await;
}
