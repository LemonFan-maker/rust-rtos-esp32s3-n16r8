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

const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";

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

#[embassy_executor::task]
async fn wifi_connect_task(wifi_ctrl: &'static mut WifiController<'static>) {
    println!("WiFi connect task started");
    println!("Target SSID: {}", WIFI_SSID);

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

    if let Err(e) = wifi_ctrl.start_async().await {
        println!("WiFi start failed: {:?}", e);
        return;
    }
    println!("WiFi started");

    println!("Connecting to AP...");
    if let Err(e) = wifi_ctrl.connect_async().await {
        println!("WiFi connect failed: {:?}", e);
        return;
    }
    // connect_async() 内部已等待 StaConnected 事件后才返回 Ok,链路此时已建立;
    // 再次 wait_for_event(StaConnected) 会因事件已被消费而永久阻塞。
    println!("StaConnected event observed by connect_async; link is up.");

    let mac = esp_radio::wifi::sta_mac();
    println!("STA MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

    match wifi_ctrl.rssi() {
        Ok(rssi) => println!("Signal strength: {} dBm", rssi),
        Err(e) => println!("Failed to get RSSI: {:?}", e),
    }

    println!("WiFi Connected Successfully!");

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
                        println!("[STATUS] Attempting reconnect...");
                        let _ = wifi_ctrl.connect_async().await;
                    }
                }
            }
            Err(e) => println!("[STATUS] Error checking connection: {:?}", e),
        }

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

    println!("RustRTOS WiFi Connect Example");
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

    println!("Starting WiFi connect task...");
    spawner.spawn(wifi_connect_task(wifi_ctrl)).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
