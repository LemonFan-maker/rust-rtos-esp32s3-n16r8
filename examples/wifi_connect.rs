#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use static_cell::StaticCell;

use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;
use rustrtos::net::tcp::{NetworkStack, StackConfig};
use rustrtos::net::wifi::{WifiController, WifiEvent};

const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";
const NET_SOCKET_SLOTS: usize = 2;

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

static WIFI_EVENT_CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>> =
    StaticCell::new();
static NET_RESOURCES: StaticCell<embassy_net::StackResources<NET_SOCKET_SLOTS>> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

async fn park_after_spawn() -> ! {
    println!("WiFi/DHCP demo halted (keeping net_task alive).");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn wifi_connect_task(
    spawner: Spawner,
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI<'static>,
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
) {
    println!("WiFi connect task started");
    println!("Target SSID: {}", WIFI_SSID);

    let mut wifi_ctrl = match WifiController::new(radio, wifi_peripheral, event_channel) {
        Ok(controller) => controller,
        Err(e) => {
            println!("WiFi controller init failed: {:?}", e);
            return;
        }
    };

    println!("Connecting to AP...");
    if let Err(e) = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await {
        println!("WiFi connect failed: {:?}", e);
        return;
    }
    println!("StaConnected event observed by connect; link is up.");

    let mac = wifi_ctrl.mac_address();
    println!(
        "STA MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    match wifi_ctrl.rssi() {
        Ok(rssi) => println!("Signal strength: {} dBm", rssi),
        Err(e) => println!("Failed to get RSSI: {:?}", e),
    }

    let device = match wifi_ctrl.take_sta() {
        Some(device) => device,
        None => {
            println!("STA interface already taken");
            return;
        }
    };

    let seed = esp_hal::rng::Rng::new().random() as u64;
    let (stack, runner) = NetworkStack::new(
        device,
        StackConfig::default(),
        NET_RESOURCES.init(embassy_net::StackResources::new()),
        seed,
    );
    if spawner.spawn(net_task(runner)).is_err() {
        println!("Failed to spawn net_task");
        return;
    }

    println!("Waiting for IP (DHCP)...");
    let ip_info = match stack.wait_for_ip().await {
        Ok(info) => info,
        Err(e) => {
            println!("Failed to get IP: {:?}", e);
            park_after_spawn().await;
        }
    };
    println!("Got IP: {}", ip_info.ip);
    println!("Netmask: {}", ip_info.netmask);
    if let Some(gateway) = ip_info.gateway {
        println!("Gateway: {}", gateway);
        wifi_ctrl.set_ip_address(ip_info.ip.octets(), gateway.octets());
    }
    for dns in &ip_info.dns_servers {
        println!("DNS: {}", dns);
    }

    while let Some(event) = wifi_ctrl.try_recv_event() {
        println!("WiFi event: {:?}", event);
    }

    println!("WiFi Connected Successfully!");

    let mut connected = true;
    loop {
        Timer::after(Duration::from_secs(5)).await;

        match wifi_ctrl.is_connected() {
            is_connected if is_connected != connected => {
                connected = is_connected;
                if connected {
                    println!("[STATUS] Reconnected!");
                    match stack.wait_for_ip().await {
                        Ok(info) => {
                            println!("[STATUS] DHCP IP: {}", info.ip);
                            if let Some(gateway) = info.gateway {
                                wifi_ctrl.set_ip_address(info.ip.octets(), gateway.octets());
                            }
                        }
                        Err(e) => println!("[STATUS] DHCP refresh failed: {:?}", e),
                    }
                } else {
                    println!("[STATUS] Disconnected!");
                    println!("[STATUS] Attempting reconnect...");
                    if let Err(e) = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await {
                        println!("[STATUS] Reconnect failed: {:?}", e);
                    }
                }
            }
            _ => {}
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
    let event_channel = WIFI_EVENT_CHANNEL.init(Channel::new());

    println!("Starting WiFi connect task...");
    spawner
        .spawn(wifi_connect_task(spawner, radio_ref, peripherals.WIFI, event_channel))
        .ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
