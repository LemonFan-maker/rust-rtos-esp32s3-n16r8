#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_time::Timer;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::timer::timg::TimerGroup;
use portable_atomic::{AtomicU8, Ordering};
use static_cell::StaticCell;

use trouble_host::prelude::*;

use rustrtos::net::ble::{esp_ble_controller, AdvertiseConfig, BleController, DisconnectReason};

const DEVICE_NAME: &str = "RustRTOS-GATT";

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

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

static LED_STATE: AtomicU8 = AtomicU8::new(0);

#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    level: u8,
}

async fn runner_pump<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            println!("[BLE] Runner error: {:?}", e);
        }
    }
}

async fn gatt_events_task<P: PacketPool>(
    conn_handle: u16,
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> (u16, DisconnectReason) {
    let level = server.battery_service.level;

    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                println!("[GATT] Disconnected: {:?}", reason);
                return (conn_handle, DisconnectReason::from_hci_status(reason.into_inner()));
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
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => println!("[GATT] Error sending response: {:?}", e),
                };
            }
            _ => {}
        }
    }
}

async fn notification_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let level = server.battery_service.level;
    let mut battery: u8 = 100;

    loop {
        Timer::after(embassy_time::Duration::from_secs(2)).await;

        battery = if battery > 0 { battery - 1 } else { 100 };

        println!("[GATT] Notifying battery level: {}%", battery);

        if level.notify(conn, &battery).await.is_err() {
            println!("[GATT] Notify error, connection may be closed");
            break;
        }
    }
}

async fn gatt_flow<'a, 's, 'v, C: Controller>(
    ble: &mut BleController<'a, C, DefaultPacketPool>,
    server: &'s Server<'v>,
) {
    let mut adv_bytes = [0u8; 31];
    let len = match AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut adv_bytes[..],
    ) {
        Ok(len) => len,
        Err(e) => {
            println!("[BLE] Encode adv data failed: {:?}", e);
            return;
        }
    };
    let config = AdvertiseConfig::default().with_adv_data(&adv_bytes[..len]);

    loop {
        if let Err(e) = ble.start_advertising(&config).await {
            println!("[BLE] Advertise error: {:?}", e);
            Timer::after(embassy_time::Duration::from_secs(1)).await;
            continue;
        }
        println!("[BLE] Advertising started");

        match ble.wait_for_connection().await {
            Ok(conn) => {
                let handle = conn.handle().raw();
                match conn.with_attribute_server(server) {
                    Ok(gatt_conn) => {
                        println!("[BLE] Connection established!");

                        let events = gatt_events_task(handle, server, &gatt_conn);
                        let notify = notification_task(server, &gatt_conn);

                        let (conn_handle, reason) = match select(events, notify).await {
                            Either::First(r) => r,
                            Either::Second(()) => (handle, DisconnectReason::Unknown),
                        };
                        println!("[BLE] Connection ended, restarting advertising...");

                        drop(gatt_conn);
                        ble.note_disconnected(conn_handle, reason);
                    }
                    Err(e) => {
                        println!("[BLE] Attribute server error: {:?}", e);
                        ble.note_disconnected(handle, DisconnectReason::LocalHostTerminated);
                    }
                }
            }
            Err(e) => {
                println!("[BLE] Accept error: {:?}", e);
                Timer::after(embassy_time::Duration::from_secs(1)).await;
            }
        }
    }
}

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

    println!("RustRTOS BLE GATT Server Example");
    println!("ESP32-S3 @ 240MHz");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    spawner.spawn(led_task(led)).ok();

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

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    println!("BLE GATT Server Active");
    println!("Device: {}", DEVICE_NAME);
    println!("Services: Battery Service (0x180F)");

    let runner = ble.take_runner().expect("runner");
    let _ = join(runner_pump(runner), gatt_flow(&mut ble, &server)).await;
}
