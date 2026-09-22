#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::mem::MaybeUninit;
use core::net::{Ipv4Addr, SocketAddrV4};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::timer::timg::TimerGroup;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;
use rustrtos::net::tcp::{NetworkStack, StackConfig, TcpClient};
use rustrtos::net::wifi::{WifiController, WifiEvent, WifiMode};

const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";

const SERVER_IP: [u8; 4] = [192, 168, 1, 10];
const SERVER_PORT: u16 = 5001;

const TCP_TEST_DURATION_SECS: u64 = 10;
const TCP_BUFFER_SIZE: usize = 1024;

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
static TCP_RX_BUF: StaticCell<[u8; TCP_BUFFER_SIZE]> = StaticCell::new();
static TCP_TX_BUF: StaticCell<[u8; TCP_BUFFER_SIZE]> = StaticCell::new();
static TX_PATTERN: [u8; TCP_BUFFER_SIZE] = [0xAA_u8; TCP_BUFFER_SIZE];

static TX_BYTES: AtomicU64 = AtomicU64::new(0);
static RX_BYTES: AtomicU64 = AtomicU64::new(0);
static TX_PACKETS: AtomicU32 = AtomicU32::new(0);
static RX_PACKETS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Default)]
struct BenchmarkResult {
    name: &'static str,
    duration_us: u64,
    tx_bytes: u64,
    rx_bytes: u64,
    throughput_kbps: u32,
    avg_latency_us: u32,
    min_latency_us: u32,
    max_latency_us: u32,
}

impl BenchmarkResult {
    fn print(&self) {
        println!("{}", self.name);
        println!("Duration:{} ms", self.duration_us / 1000);
        println!("TX bytes:{} KB", self.tx_bytes / 1024);
        println!("RX bytes:{} KB", self.rx_bytes / 1024);
        println!("Throughput:{} Kbps ({} KB/s)",
            self.throughput_kbps,
            self.throughput_kbps / 8);
        if self.avg_latency_us > 0 {
            println!("Latency avg:{} us", self.avg_latency_us);
            println!("Latency min:{} us", self.min_latency_us);
            println!("Latency max:{} us", self.max_latency_us);
        }
    }
}

fn server_addr() -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(SERVER_IP[0], SERVER_IP[1], SERVER_IP[2], SERVER_IP[3]),
        SERVER_PORT,
    )
}

async fn benchmark_tcp_throughput_tx<'a>(
    stack: embassy_net::Stack<'a>,
    rx_buf: &'a mut [u8],
    tx_buf: &'a mut [u8],
) -> BenchmarkResult {
    println!("[Benchmark] TCP TX Throughput");
    println!("Connecting to {}...", server_addr());

    let mut tcp_client = TcpClient::new(stack, rx_buf, tx_buf);

    if tcp_client.connect(server_addr()).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP TX Throughput",
            ..Default::default()
        };
    }

    println!("Connected, starting TX test for {} seconds...", TCP_TEST_DURATION_SECS);

    TX_BYTES.store(0, Ordering::Relaxed);
    TX_PACKETS.store(0, Ordering::Relaxed);

    let start = Instant::now();
    let deadline = Duration::from_secs(TCP_TEST_DURATION_SECS);

    while start.elapsed() < deadline {
        match tcp_client.write(&TX_PATTERN).await {
            Ok(sent) => {
                TX_BYTES.fetch_add(sent as u64, Ordering::Relaxed);
                TX_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => break,
        }
    }

    let duration = start.elapsed();
    let total_bytes = TX_BYTES.load(Ordering::Relaxed);
    let total_packets = TX_PACKETS.load(Ordering::Relaxed);

    let throughput_kbps = if duration.as_micros() > 0 {
        ((total_bytes * 8 * 1_000_000) / duration.as_micros()) as u32 / 1000
    } else {
        0
    };

    let _ = tcp_client.close().await;
    drop(tcp_client);

    println!("TX Test complete:");
    println!("Sent: {} KB in {} packets", total_bytes / 1024, total_packets);

    BenchmarkResult {
        name: "TCP TX Throughput",
        duration_us: duration.as_micros(),
        tx_bytes: total_bytes,
        throughput_kbps,
        ..Default::default()
    }
}

async fn benchmark_tcp_throughput_rx<'a>(
    stack: embassy_net::Stack<'a>,
    rx_buf: &'a mut [u8],
    tx_buf: &'a mut [u8],
) -> BenchmarkResult {
    println!("[Benchmark] TCP RX Throughput");
    println!("Note: Requires iperf client sending data to this device");

    let mut tcp_client = TcpClient::new(stack, rx_buf, tx_buf);

    if tcp_client.connect(server_addr()).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP RX Throughput",
            ..Default::default()
        };
    }

    println!("Connected, waiting for data for {} seconds...", TCP_TEST_DURATION_SECS);

    RX_BYTES.store(0, Ordering::Relaxed);
    RX_PACKETS.store(0, Ordering::Relaxed);

    let start = Instant::now();
    let deadline = Duration::from_secs(TCP_TEST_DURATION_SECS);

    let mut sink = [0u8; 512];
    while start.elapsed() < deadline {
        match tcp_client.read_timeout(&mut sink, Duration::from_millis(500)).await {
            Ok(received) if received > 0 => {
                RX_BYTES.fetch_add(received as u64, Ordering::Relaxed);
                RX_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
            Ok(_) => break,
            Err(_) => continue,
        }
    }

    let duration = start.elapsed();
    let total_bytes = RX_BYTES.load(Ordering::Relaxed);
    let total_packets = RX_PACKETS.load(Ordering::Relaxed);

    let throughput_kbps = if duration.as_micros() > 0 {
        ((total_bytes * 8 * 1_000_000) / duration.as_micros()) as u32 / 1000
    } else {
        0
    };

    let _ = tcp_client.close().await;
    drop(tcp_client);

    println!("RX Test complete:");
    println!("Received: {} KB in {} packets", total_bytes / 1024, total_packets);

    BenchmarkResult {
        name: "TCP RX Throughput",
        duration_us: duration.as_micros(),
        rx_bytes: total_bytes,
        throughput_kbps,
        ..Default::default()
    }
}

async fn benchmark_tcp_latency<'a>(
    stack: embassy_net::Stack<'a>,
    rx_buf: &'a mut [u8],
    tx_buf: &'a mut [u8],
) -> BenchmarkResult {
    println!("[Benchmark] TCP Latency (Echo)");

    let mut tcp_client = TcpClient::new(stack, rx_buf, tx_buf);

    if tcp_client.connect(server_addr()).await.is_err() {
        println!("TCP connect failed!");
        return BenchmarkResult {
            name: "TCP Latency",
            ..Default::default()
        };
    }

    const PING_COUNT: u32 = 100;
    const PING_SIZE: usize = 64;

    println!("Running {} ping-pong tests with {} byte packets...", PING_COUNT, PING_SIZE);

    let ping_data = [0x55_u8; PING_SIZE];
    let mut pong_data = [0u8; PING_SIZE];

    let mut total_latency_us = 0u64;
    let mut min_latency_us = u32::MAX;
    let mut max_latency_us = 0u32;
    let mut successful_pings = 0u32;

    for i in 0..PING_COUNT {
        let start = Instant::now();

        if tcp_client.write(&ping_data).await.is_err() {
            break;
        }

        let mut got = 0usize;
        let mut ok = true;
        while got < PING_SIZE {
            match tcp_client
                .read_timeout(&mut pong_data[got..], Duration::from_secs(1))
                .await
            {
                Ok(0) => {
                    ok = false;
                    break;
                }
                Ok(n) => got += n,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;
        }

        let latency_us = start.elapsed().as_micros() as u32;

        total_latency_us += latency_us as u64;
        min_latency_us = min_latency_us.min(latency_us);
        max_latency_us = max_latency_us.max(latency_us);
        successful_pings += 1;

        if (i + 1) % 20 == 0 {
            println!("Progress: {}/{}", i + 1, PING_COUNT);
        }
    }

    let _ = tcp_client.close().await;
    drop(tcp_client);

    let avg_latency_us = if successful_pings > 0 {
        (total_latency_us / successful_pings as u64) as u32
    } else {
        0
    };

    println!("Latency test complete: {}/{} successful", successful_pings, PING_COUNT);

    BenchmarkResult {
        name: "TCP Latency",
        duration_us: total_latency_us,
        avg_latency_us,
        min_latency_us: if min_latency_us == u32::MAX { 0 } else { min_latency_us },
        max_latency_us,
        ..Default::default()
    }
}

#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn benchmark_task(
    spawner: Spawner,
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI<'static>,
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
) {
    println!("RustRTOS Network Benchmark Suite");
    println!("ESP32-S3 @ 240MHz");

    let mut results: heapless::Vec<BenchmarkResult, 4> = heapless::Vec::new();

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

    println!("Running benchmark 1/4: WiFi Connection + DHCP Time");
    println!("Connecting to '{}'...", WIFI_SSID);

    let connect_start = Instant::now();
    let connect_result = wifi_ctrl.connect(WIFI_SSID, WIFI_PASSWORD).await;
    let connect_time = connect_start.elapsed();

    if connect_result.is_err() {
        println!("Connection failed!");
        let _ = results.push(BenchmarkResult {
            name: "WiFi Connect + DHCP",
            duration_us: connect_time.as_micros(),
            ..Default::default()
        });
        print_summary(&results);
        return;
    }
    println!("WiFi connected in {} ms", connect_time.as_millis());

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

    let dhcp_start = Instant::now();
    let ip_info = stack.wait_for_ip().await;
    let dhcp_time = dhcp_start.elapsed();

    match &ip_info {
        Ok(info) => {
            println!("Got IP: {} in {} ms", info.ip, dhcp_time.as_millis());
            if let Some(gw) = info.gateway {
                wifi_ctrl.set_ip_address(info.ip.octets(), gw.octets());
            }
        }
        Err(e) => println!("DHCP failed: {:?}", e),
    }

    let total_time = connect_time + dhcp_time;
    let _ = results.push(BenchmarkResult {
        name: "WiFi Connect + DHCP",
        duration_us: total_time.as_micros(),
        avg_latency_us: connect_time.as_micros() as u32,
        min_latency_us: connect_time.as_micros() as u32,
        max_latency_us: total_time.as_micros() as u32,
        ..Default::default()
    });

    if ip_info.is_err() {
        print_summary(&results);
        // net_task 已 spawn 且永久存活,此处 return 会 drop wifi_ctrl 触发
        // wifi_deinit,runner 后续 dispatch 访问已释放驱动(LoadProhibited)。park。
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    }

    let stack_handle = stack.stack();

    let rx_buf = TCP_RX_BUF.init([0u8; TCP_BUFFER_SIZE]);
    let tx_buf = TCP_TX_BUF.init([0u8; TCP_BUFFER_SIZE]);

    println!("Running benchmark 2/4: TCP TX Throughput");
    let result = benchmark_tcp_throughput_tx(stack_handle, rx_buf, tx_buf).await;
    let _ = results.push(result);

    Timer::after(Duration::from_secs(2)).await;

    println!("Running benchmark 3/4: TCP RX Throughput");
    let result = benchmark_tcp_throughput_rx(stack_handle, rx_buf, tx_buf).await;
    let _ = results.push(result);

    Timer::after(Duration::from_secs(2)).await;

    println!("Running benchmark 4/4: TCP Latency");
    let result = benchmark_tcp_latency(stack_handle, rx_buf, tx_buf).await;
    let _ = results.push(result);

    print_summary(&results);

    println!("Benchmark Suite Complete!");

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

fn print_summary(results: &heapless::Vec<BenchmarkResult, 4>) {
    println!("BENCHMARK RESULTS SUMMARY");

    for result in results {
        result.print();
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("RustRTOS Network Benchmark");
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

    spawner.spawn(benchmark_task(
        spawner,
        radio,
        peripherals.WIFI,
        event_channel,
    )).ok();

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
