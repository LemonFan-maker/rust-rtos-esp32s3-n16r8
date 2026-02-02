//! BLE GATT Server 示例 - 使用真实 trouble-host API
//!
//! 演示如何创建一个 BLE GATT 服务端，提供 Battery Service。
//!
//! # 功能
//! - Battery Service (0x180F)
//! - 电池电量特征值 (只读 + 通知)
//!
//! # 运行
//! ```bash
//! cargo run --example ble_gatt_server --features ble,dev --release
//! ```

#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::Timer;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU8, Ordering};
use static_cell::StaticCell;

// esp-radio BLE 控制器
use esp_radio::ble::controller::BleConnector;

// trouble-host BLE 协议栈
use trouble_host::prelude::*;

// ===== 配置 =====
const DEVICE_NAME: &str = "RustRTOS-GATT";

// ===== 堆初始化 =====
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

// 共享 LED 状态
static LED_STATE: AtomicU8 = AtomicU8::new(0);

// GATT Server 定义
#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

// Battery Service
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// Battery Level (0-100%)
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    level: u8,
}

/// 运行 BLE 协议栈任务
async fn ble_task<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            println!("[BLE] Runner error: {:?}", e);
        }
    }
}

/// 处理 GATT 事件
async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                println!("[GATT] Disconnected: {:?}", reason);
                break;
            }
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(ev) => {
                        if ev.handle() == level.handle {
                            let value = server.get(&level);
                            println!("[GATT] Read battery level: {:?}", value);
                        }
                    }
                    GattEvent::Write(ev) => {
                        println!("[GATT] Write event: handle={}, data={:?}", 
                            ev.handle(), ev.data());
                    }
                    _ => {}
                };
                // 发送响应
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => println!("[GATT] Error sending response: {:?}", e),
                };
            }
            _ => {}
        }
    }
    Ok(())
}

/// 发送电池电量通知任务
async fn notification_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let level = server.battery_service.level;
    let mut battery: u8 = 100;
    
    loop {
        Timer::after(embassy_time::Duration::from_secs(2)).await;
        
        // 模拟电池放电
        battery = if battery > 0 { battery - 1 } else { 100 };
        
        println!("[GATT] Notifying battery level: {}%", battery);
        
        if level.notify(conn, &battery).await.is_err() {
            println!("[GATT] Notify error, connection may be closed");
            break;
        }
    }
}

/// 广播并等待连接
async fn advertise<'values, 'server, C: Controller>(
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv_data = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),  // Battery Service UUID
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut adv_data[..],
    )?;
    
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    
    println!("[BLE] Advertising started");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    println!("[BLE] Connection established!");
    Ok(conn)
}

/// 主 BLE GATT 任务
async fn ble_gatt_server<C: Controller>(controller: C) {
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
        runner,
        ..
    } = stack.build();

    // 创建 GATT 服务器
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    println!("\n=========================================");
    println!("   BLE GATT Server Active");
    println!("   Device: {}", DEVICE_NAME);
    println!("   Services: Battery Service (0x180F)");
    println!("=========================================\n");

    // 运行 BLE 协议栈和 GATT 服务
    let _ = join(
        ble_task(runner),
        async {
            loop {
                match advertise(&mut peripheral, &server).await {
                    Ok(conn) => {
                        // 连接后运行任务
                        let events = gatt_events_task(&server, &conn);
                        let notify = notification_task(&server, &conn);
                        
                        // 任意一个任务结束则返回广播
                        select(events, notify).await;
                        println!("[BLE] Connection ended, restarting advertising...");
                    }
                    Err(e) => {
                        println!("[BLE] Advertise error: {:?}", e);
                        Timer::after(embassy_time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    ).await;
}

/// LED 控制任务
#[embassy_executor::task]
async fn led_task(mut led: Output<'static>) {
    println!("LED task started");
    
    let mut last_state = 0u8;
    
    loop {
        let current_state = LED_STATE.load(Ordering::Relaxed);
        
        if current_state != last_state {
            if current_state == 1 {
                led.set_high();
                println!("[LED] ON");
            } else {
                led.set_low();
                println!("[LED] OFF");
            }
            last_state = current_state;
        }
        
        Timer::after(embassy_time::Duration::from_millis(50)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();
    
    let peripherals = esp_hal::init(esp_hal::Config::default());
    
    println!("=========================================");
    println!("   RustRTOS BLE GATT Server Example");
    println!("   ESP32-S3 @ 240MHz");
    println!("=========================================");

    // 初始化时钟
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // 初始化 LED
    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    spawner.spawn(led_task(led)).ok();

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

    // 运行 BLE GATT 服务器
    ble_gatt_server(controller).await;
}
