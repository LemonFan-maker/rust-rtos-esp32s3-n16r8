#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;

use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;
use rustrtos::net::tcp::{NetworkStack, StackConfig, TcpClient};
use rustrtos::net::wifi::{WifiController, WifiEvent, WifiMode};

const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";

const SERVER_HOST: &str = "baidu.com";
const SERVER_PORT: u16 = 80;

const NET_SOCKET_SLOTS: usize = 6;

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

static WIFI_EVENT_CHANNEL: StaticCell<Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>> = StaticCell::new();
static NET_RESOURCES: StaticCell<embassy_net::StackResources<NET_SOCKET_SLOTS>> = StaticCell::new();
static TCP_RX_BUF: StaticCell<[u8; 2048]> = StaticCell::new();
static TCP_TX_BUF: StaticCell<[u8; 2048]> = StaticCell::new();

const HTTP_REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: baidu.com\r\nConnection: close\r\n\r\n";

#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

// net_task(runner) 一旦 spawn 就永久存活并持有 embassy-net 的 iface;
// 若本任务提前 return,局部 wifi_ctrl 被 drop 触发 esp-radio wifi_deinit,
// runner 后续 dispatch 会访问已释放驱动(实测 LoadProhibited@esp_wifi_internal_tx)。
// 因此 spawn 之后的所有失败出口必须 park 而非 return。
async fn park_after_spawn() -> ! {
    println!("TCP client demo halted (keeping WiFi/net_task alive).");
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn tcp_client_task(
    spawner: Spawner,
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI<'static>,
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
) {
    println!("TCP client task started");

    let mut wifi_ctrl = match WifiController::new(radio, wifi_peripheral, event_channel) {
        Ok(c) => c,
        Err(e) => {
            println!("WiFi controller init failed: {:?}", e);
            return;
        }
    };

    if let Err(e) = wifi_ctrl.set_mode(WifiMode::Sta).await {
        println!("Set mode failed: {:?}", e);
        return;
    }

    match wifi_ctrl.scan().await {
        Ok(aps) => println!("Scan found {} networks", aps.len()),
        Err(e) => println!("Scan failed: {:?}", e),
    }

    println!("Connecting to WiFi '{}'...", WIFI_SSID);
    if let Err(e) = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await {
        println!("WiFi connect failed: {:?}", e);
        return;
    }

    let mac = wifi_ctrl.mac_address();
    println!(
        "WiFi connected. STA MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    if let Ok(rssi) = wifi_ctrl.rssi() {
        println!("RSSI: {} dBm", rssi);
    }

    let device = match wifi_ctrl.take_sta() {
        Some(d) => d,
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
    if let Some(gw) = ip_info.gateway {
        println!("Gateway: {}", gw);
        wifi_ctrl.set_ip_address(ip_info.ip.octets(), gw.octets());
    }
    for dns in &ip_info.dns_servers {
        println!("DNS: {}", dns);
    }

    while let Some(event) = wifi_ctrl.try_recv_event() {
        println!("WiFi event: {:?}", event);
    }

    let server_ip = match stack.dns_resolve(SERVER_HOST).await {
        Ok(ip) => {
            println!("DNS {} -> {:?}", SERVER_HOST, ip.octets());
            ip
        }
        Err(e) => {
            println!("DNS resolve failed: {:?}", e);
            park_after_spawn().await;
        }
    };

    println!("Connecting to {}:{}...", SERVER_HOST, SERVER_PORT);

    let rx_buf = TCP_RX_BUF.init([0u8; 2048]);
    let tx_buf = TCP_TX_BUF.init([0u8; 2048]);
    let mut tcp_client = TcpClient::new(stack.stack(), rx_buf, tx_buf);

    if let Err(e) = tcp_client.connect_to(server_ip, SERVER_PORT).await {
        println!("TCP connect failed: {:?}", e);
        park_after_spawn().await;
    }
    println!("TCP connected! Local port: {}", tcp_client.local_port());

    println!("Sending HTTP request...");
    if let Ok(req_str) = core::str::from_utf8(HTTP_REQUEST) {
        for line in req_str.lines() {
            println!("> {}", line);
        }
    }

    match tcp_client.write(HTTP_REQUEST).await {
        Ok(sent) => println!("Sent {} bytes", sent),
        Err(e) => {
            println!("Send failed: {:?}", e);
            park_after_spawn().await;
        }
    }

    println!("Waiting for response...");
    let mut rx_slice = [0u8; 1024];
    let mut total_received = 0usize;

    loop {
        match tcp_client.read_timeout(&mut rx_slice, Duration::from_secs(5)).await {
            Ok(0) => {
                break;
            }
            Ok(len) => {
                total_received += len;

                if let Ok(response) = core::str::from_utf8(&rx_slice[..len]) {
                    for line in response.lines().take(10) {
                        println!("< {}", line);
                    }
                    if response.lines().count() > 10 {
                        println!("< ... (truncated)");
                    }
                }
            }
            Err(rustrtos::net::tcp::NetworkError::Timeout) => break,
            Err(e) => {
                println!("Read error: {:?}", e);
                break;
            }
        }
    }

    println!("Total received: {} bytes", total_received);

    println!("Closing connection...");
    if let Err(e) = tcp_client.close().await {
        println!("Close error: {:?}", e);
    }

    println!("TCP Client Demo Complete!");

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("RustRTOS TCP Client Example");
    println!("ESP32-S3 @ 240MHz");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio = match esp_radio::init() {
        Ok(ctrl) => {
            println!("esp-radio initialized successfully");
            RADIO_CONTROLLER.init(ctrl)
        }
        Err(e) => {
            println!("esp-radio init failed: {:?}", e);
            loop { core::hint::spin_loop(); }
        }
    };

    let event_channel = WIFI_EVENT_CHANNEL.init(Channel::new());

    spawner.spawn(tcp_client_task(
        spawner,
        radio,
        peripherals.WIFI,
        event_channel,
    )).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
