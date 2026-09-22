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

use trouble_host::prelude::*;

use rustrtos::net::ble::{esp_ble_controller, AdvertiseConfig, BleController, DisconnectReason};

const DEVICE_NAME: &[u8] = b"RustRTOS-BLE";

fn init_heap() {
    const HEAP_SIZE: usize = 72 * 1024;
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            core::ptr::addr_of_mut!(HEAP) as *mut u8,
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

#[cfg(feature = "dev")]
use esp_println::println;

#[cfg(not(feature = "dev"))]
macro_rules! println {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(feature = "dev")]
use esp_backtrace as _;

#[cfg(not(feature = "dev"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

async fn runner_pump<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            println!("[BLE] Runner error: {:?}", e);
        }
    }
}

async fn advertise_flow<'a, C: Controller, P: PacketPool>(
    ble: &mut BleController<'a, C, P>,
) {
    let mut adv_bytes = [0u8; 31];
    let len = match AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(DEVICE_NAME),
        ],
        &mut adv_bytes[..],
    ) {
        Ok(len) => len,
        Err(e) => {
            println!("[BLE] Encode adv data failed: {:?}", e);
            return;
        }
    };

    let config = AdvertiseConfig::default()
        .with_adv_data(&adv_bytes[..len])
        .with_interval_ms(100);

    loop {
        println!("[BLE] Starting advertising...");

        if let Err(e) = ble.start_advertising(&config).await {
            println!("[BLE] Advertising error: {:?}", e);
            Timer::after(embassy_time::Duration::from_secs(1)).await;
            continue;
        }

        println!("[BLE] Advertising started, waiting for connection...");

        match ble.wait_for_connection().await {
            Ok(conn) => {
                println!("[BLE] Connection established!");
                println!("[BLE] Peer: {:?}", conn.peer_address());

                let handle = conn.handle().raw();
                while conn.is_connected() {
                    Timer::after(embassy_time::Duration::from_secs(1)).await;
                }
                println!("[BLE] Connection closed");
                drop(conn);

                ble.note_disconnected(handle, DisconnectReason::Unknown);
            }
            Err(e) => {
                println!("[BLE] Accept error: {:?}", e);
                Timer::after(embassy_time::Duration::from_secs(1)).await;
            }
        }
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    init_heap();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("RustRTOS BLE Advertise Example");
    println!("ESP32-S3 @ 240MHz");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

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

    static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_ref = RADIO_CONTROLLER.init(radio_controller);

    let controller = match esp_ble_controller(radio_ref, peripherals.BT) {
        Ok(c) => {
            println!("BLE controller initialized");
            c
        }
        Err(e) => {
            println!("BLE controller init failed: {:?}", e);
            loop { core::hint::spin_loop(); }
        }
    };

    let address: Address = Address::random([0x41, 0x5A, 0xE3, 0x1E, 0x83, 0xE7]);
    println!("BLE Address: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        0x41, 0x5A, 0xE3, 0x1E, 0x83, 0xE7);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address);

    let mut ble = BleController::new(stack.build(), address);
    println!("BLE Advertising Active");
    println!("Device Name: {}", core::str::from_utf8(DEVICE_NAME).unwrap_or("RustRTOS"));

    let runner = ble.take_runner().expect("runner");
    let _ = join(runner_pump(runner), advertise_flow(&mut ble)).await;
}
