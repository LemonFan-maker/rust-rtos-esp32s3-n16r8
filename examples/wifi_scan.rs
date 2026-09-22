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

use esp_radio::wifi::{
    ModeConfig, WifiController, ClientConfig,
};

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

#[embassy_executor::task]
async fn wifi_scan_task(wifi_ctrl: &'static mut WifiController<'static>) {
    println!("WiFi scan task started");

    let station_config = ModeConfig::Client(ClientConfig::default());

    if let Err(e) = wifi_ctrl.set_config(&station_config) {
        println!("WiFi set config failed: {:?}", e);
        return;
    }

    if let Err(e) = wifi_ctrl.start_async().await {
        println!("WiFi start failed: {:?}", e);
        return;
    }

    println!("WiFi started successfully");

    let mut scan_count = 0u32;

    loop {
        scan_count += 1;
        println!("WiFi Scan #{}", scan_count);

        match wifi_ctrl.scan_with_config_async(Default::default()).await {
            Ok(results) => {
                if results.is_empty() {
                    println!("No WiFi networks found");
                } else {
                    println!("Found {} networks:", results.len());
                    println!("{:<32} {:>6} {:>4} {:>8}", "SSID", "RSSI", "CH", "Auth");

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

        println!("Next scan in 10 seconds...");
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("RustRTOS WiFi Scan Example");
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

    println!("Starting WiFi scan task...");
    spawner.spawn(wifi_scan_task(wifi_ctrl)).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
