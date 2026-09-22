use heapless::String;

#[derive(Clone)]
pub struct NetworkConfig {
    pub wifi_ssid: String<32>,
    pub wifi_password: String<64>,
    pub ble_device_name: String<32>,
    pub dhcp_enabled: bool,
    pub static_ip: Option<[u8; 4]>,
    pub gateway: Option<[u8; 4]>,
    pub dns_server: Option<[u8; 4]>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            ble_device_name: String::try_from("RustRTOS").unwrap_or_default(),
            dhcp_enabled: true,
            static_ip: None,
            gateway: None,
            dns_server: None,
        }
    }
}

impl NetworkConfig {
    pub const fn new() -> Self {
        Self {
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            ble_device_name: String::new(),
            dhcp_enabled: true,
            static_ip: None,
            gateway: None,
            dns_server: None,
        }
    }

    pub fn with_wifi_credentials(mut self, ssid: &str, password: &str) -> Self {
        let _ = self.wifi_ssid.clear();
        let _ = self.wifi_ssid.push_str(ssid);
        let _ = self.wifi_password.clear();
        let _ = self.wifi_password.push_str(password);
        self
    }

    pub fn with_ble_name(mut self, name: &str) -> Self {
        let _ = self.ble_device_name.clear();
        let _ = self.ble_device_name.push_str(name);
        self
    }

    pub fn with_static_ip(mut self, ip: [u8; 4], gateway: [u8; 4], dns: [u8; 4]) -> Self {
        self.dhcp_enabled = false;
        self.static_ip = Some(ip);
        self.gateway = Some(gateway);
        self.dns_server = Some(dns);
        self
    }
}

pub const WIFI_CONNECT_TIMEOUT_MS: u32 = 30_000;

pub const WIFI_SCAN_TIMEOUT_MS: u32 = 10_000;

pub const WIFI_RECONNECT_INTERVAL_MS: u32 = 5_000;

pub const WIFI_MAX_RECONNECT_ATTEMPTS: u32 = 10;

pub const WIFI_EVENT_QUEUE_SIZE: usize = 8;

pub const WIFI_MAX_SCAN_RESULTS: usize = 16;

pub const BLE_ADV_INTERVAL_FAST_MS: u32 = 100;

pub const BLE_ADV_INTERVAL_SLOW_MS: u32 = 1000;

pub const BLE_CONN_INTERVAL_MIN: u16 = 6;

pub const BLE_CONN_INTERVAL_MAX: u16 = 24;

pub const BLE_SLAVE_LATENCY: u16 = 0;

pub const BLE_SUPERVISION_TIMEOUT: u16 = 400;

pub const BLE_MTU_SIZE: u16 = 247;

pub const BLE_MAX_CONNECTIONS: usize = 3;

pub const BLE_EVENT_QUEUE_SIZE: usize = 8;

pub const TCP_RX_BUFFER_SIZE: usize = 4096;

pub const TCP_TX_BUFFER_SIZE: usize = 4096;

pub const UDP_RX_BUFFER_SIZE: usize = 2048;

pub const UDP_TX_BUFFER_SIZE: usize = 2048;

pub const MAX_TCP_SOCKETS: usize = 4;

pub const MAX_UDP_SOCKETS: usize = 4;

pub const DNS_CACHE_SIZE: usize = 4;

pub const DHCP_TIMEOUT_SECS: u32 = 30;

pub const TCP_CONNECT_TIMEOUT_SECS: u32 = 10;

pub const TCP_KEEPALIVE_INTERVAL_SECS: u32 = 60;

pub const ETHERNET_MTU: usize = 1514;

pub const IP_MTU: usize = 1500;

pub const NET_BUFFER_POOL_SIZE: usize = 16;

pub const NET_BUFFER_SIZE: usize = 1536;
