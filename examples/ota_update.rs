#![no_std]
#![no_main]

extern crate alloc;

use esp_alloc as _;

esp_bootloader_esp_idf::esp_app_desc!();

use core::fmt::Write as _;
use core::mem::MaybeUninit;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::timer::timg::TimerGroup;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;

use rustrtos::fs::{FlashConfig, FlashStorage};
use rustrtos::net::config::WIFI_EVENT_QUEUE_SIZE;
use rustrtos::net::tcp::{NetworkStack, StackConfig, TcpClient};
use heapless::String;
use rustrtos::net::wifi::{WifiController, WifiEvent, WifiMode};
use rustrtos::ota::{mark_current_valid, validate_image_header, OtaSession};
use esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN;

const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";
const OTA_HOST: &str = "192.168.1.10";
const OTA_PORT: u16 = 8000;
const OTA_PATH: &str = "/rustrtos.bin";
/// 期望镜像SHA-256(十六进制小写)。留空则跳过完整性校验, 仅依赖ESP镜像魔数
/// 与分区状态机; 生产部署应填写构建流水线对.bin产物计算的摘要。
const OTA_EXPECTED_SHA256: &str = "";

/// 解析64位十六进制字符串为32字节摘要; 非法输入返回None。
fn parse_sha256_hex(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    for (slot, pair) in digest.iter_mut().zip(bytes.chunks_exact(2)) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        *slot = (hi * 16 + lo) as u8;
    }
    Some(digest)
}

const NET_SOCKET_SLOTS: usize = 4;

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

static NET_RESOURCES: StaticCell<embassy_net::StackResources<NET_SOCKET_SLOTS>> = StaticCell::new();
static TCP_RX_BUF: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_TX_BUF: StaticCell<[u8; 4096]> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(
    mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>,
) -> ! {
    runner.run().await
}

// net_task(runner) permanently owns the embassy-net interface after it is spawned.
// Returning from ota_task would drop the Wi-Fi controller and deinitialize the radio
// while the runner can still dispatch, which has reproduced a ProCpu LoadProhibited.
async fn park_after_spawn() -> ! {
    println!("OTA task halted (keeping WiFi/net_task alive).");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    for line in headers.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(prefix) = line.get(..15) else {
            continue;
        };
        if !prefix.eq_ignore_ascii_case(b"content-length:") {
            continue;
        }

        let mut value = &line[15..];
        while let Some((first, rest)) = value.split_first() {
            if !first.is_ascii_whitespace() {
                break;
            }
            value = rest;
        }

        let mut length = 0usize;
        for byte in value {
            if !byte.is_ascii_digit() {
                break;
            }
            length = length.checked_mul(10)?.checked_add((byte - b'0') as usize)?;
        }
        return Some(length);
    }
    None
}

#[embassy_executor::task]
async fn ota_task(
    spawner: Spawner,
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI<'static>,
    event_channel: &'static Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    flash: esp_hal::peripherals::FLASH<'static>,
) {
    println!("OTA update task started");

    let mut wifi = match WifiController::new(radio, wifi_peripheral, event_channel) {
        Ok(controller) => controller,
        Err(error) => {
            println!("WiFi controller init failed: {:?}", error);
            return;
        }
    };
    if wifi.set_mode(WifiMode::Sta).await.is_err()
        || wifi.connect(WIFI_SSID, WIFI_PASSWORD).await.is_err()
    {
        println!("WiFi connection failed");
        return;
    }

    let device = match wifi.take_sta() {
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
        println!("Failed to spawn network runner");
        return;
    }

    let ip = match stack.wait_for_ip().await {
        Ok(info) => info,
        Err(error) => {
            println!("DHCP failed: {:?}", error);
            park_after_spawn().await;
        }
    };
    println!("OTA network ready at {}", ip.ip);
    let mut flash = FlashStorage::new(flash, FlashConfig::whole_flash());
    if let Err(error) = flash.init() {
        println!("Whole-flash storage init failed: {}", error);
        park_after_spawn().await;
    }
    let mut partition_table = [0u8; PARTITION_TABLE_MAX_LEN];
    match mark_current_valid(&mut flash, &mut partition_table) {
        Ok(true) => println!("Confirmed the pending OTA image after WiFi/DHCP self-check"),
        Ok(false) => println!("No pending OTA confirmation"),
        Err(error) => {
            println!("OTA boot-state check failed: {}", error);
            park_after_spawn().await;
        }
    }

    let server_ip = match stack.dns_resolve(OTA_HOST).await {
        Ok(ip) => ip,
        Err(error) => {
            println!("OTA host resolution failed: {:?}", error);
            park_after_spawn().await;
        }
    };
    let mut client = TcpClient::new(
        stack.stack(),
        TCP_RX_BUF.init([0; 4096]),
        TCP_TX_BUF.init([0; 4096]),
    );
    if client.connect_to(server_ip, OTA_PORT).await.is_err() {
        println!("OTA TCP connection failed");
        park_after_spawn().await;
    }
    let mut request: String<192> = String::new();
    if write!(
        &mut request,
        "GET {} HTTP/1.1\r\nHost: {}\r\nX-Firmware-Version: {}\r\nConnection: close\r\n\r\n",
        OTA_PATH,
        OTA_HOST,
        env!("CARGO_PKG_VERSION"),
    )
    .is_err()
        || client.write(request.as_bytes()).await.is_err()
    {
        println!("OTA HTTP request failed");
        park_after_spawn().await;
    }

    let mut response = [0u8; 2048];
    let mut response_len = 0usize;
    let no_update;
    let body_start;
    let image_len;
    loop {
        if response_len == response.len() {
            println!("OTA response headers are too large");
            park_after_spawn().await;
        }
        let read = match client
            .read_timeout(&mut response[response_len..], Duration::from_secs(10))
            .await
        {
            Ok(read) if read != 0 => read,
            _ => {
                println!("OTA response header read failed");
                park_after_spawn().await;
            }
        };
        response_len += read;
        if let Some(end) = find_header_end(&response[..response_len]) {
            let headers = &response[..end];
            let is_204 = headers.starts_with(b"HTTP/1.1 204 ")
                || headers.starts_with(b"HTTP/1.0 204 ");
            let is_200 = headers.starts_with(b"HTTP/1.1 200 ")
                || headers.starts_with(b"HTTP/1.0 200 ");
            if !is_204 && !is_200 {
                println!("OTA server returned an unsupported status");
                park_after_spawn().await;
            }
            no_update = is_204;
            body_start = end + 4;
            image_len = if no_update {
                0
            } else {
                match parse_content_length(headers) {
                    Some(length) => length,
                    None => {
                        println!("OTA response has no Content-Length");
                        park_after_spawn().await;
                    }
                }
            };
            break;
        }
    }

    if no_update {
        println!("No OTA update available for version {}", env!("CARGO_PKG_VERSION"));
        park_after_spawn().await;
    }
    if image_len == 0 || image_len % 4 != 0 {
        println!("OTA image length must be non-zero and 4-byte aligned");
        park_after_spawn().await;
    }

    let mut first_body = [0u8; 2048];
    let available = response_len
        .saturating_sub(body_start)
        .min(image_len)
        .min(first_body.len());
    first_body[..available].copy_from_slice(&response[body_start..body_start + available]);
    let first_len = if available != 0 {
        available
    } else {
        let read_len = image_len.min(first_body.len());
        match client
            .read_timeout(&mut first_body[..read_len], Duration::from_secs(10))
            .await
        {
            Ok(read) if read != 0 => read,
            _ => {
                println!("OTA image body is missing");
                park_after_spawn().await;
            }
        }
    };
    if let Err(error) = validate_image_header(&first_body[..first_len]) {
        println!("OTA image rejected before flash erase: {}", error);
        park_after_spawn().await;
    }

    let expected_digest = parse_sha256_hex(OTA_EXPECTED_SHA256);
    if !OTA_EXPECTED_SHA256.is_empty() && expected_digest.is_none() {
        println!("OTA_EXPECTED_SHA256 is not 64 hex digits; aborting");
        park_after_spawn().await;
    }
    let mut ota = match match expected_digest {
        Some(digest) => OtaSession::begin_verified(&mut flash, &mut partition_table, image_len, digest),
        None => OtaSession::begin(&mut flash, &mut partition_table, image_len),
    } {
        Ok(session) => session,
        Err(error) => {
            println!("OTA session init failed: {}", error);
            park_after_spawn().await;
        }
    };
    if let Err(error) = ota.erase_target() {
        println!("OTA target erase failed: {}", error);
        park_after_spawn().await;
    }
    if ota.push(&first_body[..first_len]).is_err() {
        println!("OTA image write failed");
        park_after_spawn().await;
    }

    let mut received = first_len;
    let mut buffer = [0u8; 2048];
    while received < image_len {
        let read_len = (image_len - received).min(buffer.len());
        let read = match client
            .read_timeout(&mut buffer[..read_len], Duration::from_secs(10))
            .await
        {
            Ok(read) if read != 0 => read,
            _ => {
                println!("OTA image stream ended early");
                park_after_spawn().await;
            }
        };
        if ota.push(&buffer[..read]).is_err() {
            println!("OTA image write failed");
            park_after_spawn().await;
        }
        received += read;
        println!("OTA progress: {}/{} bytes", received, image_len);
    }

    match ota.finish() {
        Ok(result) => {
            println!("OTA image written to {:?} ({} bytes); rebooting", result.slot, result.bytes_written);
            esp_hal::system::software_reset();
        }
        Err(error) => {
            println!("OTA activation failed: {}", error);
            park_after_spawn().await;
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    init_heap();
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let radio = match esp_radio::init() {
        Ok(radio) => radio,
        Err(error) => {
            println!("esp-radio init failed: {:?}", error);
            loop { core::hint::spin_loop(); }
        }
    };
    static RADIO: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio = RADIO.init(radio);
    static EVENTS: StaticCell<Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>> =
        StaticCell::new();
    let events = EVENTS.init(Channel::new());

    spawner
        .spawn(ota_task(spawner, radio, peripherals.WIFI, events, peripherals.FLASH))
        .ok();
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
