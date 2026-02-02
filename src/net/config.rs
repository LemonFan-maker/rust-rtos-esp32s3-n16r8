//! 网络配置常量
//!
//! 定义 WiFi、BLE 和 TCP/IP 网络的默认配置参数。

use heapless::String;

/// 网络配置结构
#[derive(Clone)]
pub struct NetworkConfig {
    /// WiFi SSID (最大 32 字节)
    pub wifi_ssid: String<32>,
    /// WiFi 密码 (最大 64 字节)
    pub wifi_password: String<64>,
    /// BLE 设备名称 (最大 32 字节)
    pub ble_device_name: String<32>,
    /// DHCP 启用
    pub dhcp_enabled: bool,
    /// 静态 IP 地址 (如果不使用 DHCP)
    pub static_ip: Option<[u8; 4]>,
    /// 网关地址
    pub gateway: Option<[u8; 4]>,
    /// DNS 服务器
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
    /// 创建新的网络配置
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

    /// 设置 WiFi 凭据
    pub fn with_wifi_credentials(mut self, ssid: &str, password: &str) -> Self {
        let _ = self.wifi_ssid.clear();
        let _ = self.wifi_ssid.push_str(ssid);
        let _ = self.wifi_password.clear();
        let _ = self.wifi_password.push_str(password);
        self
    }

    /// 设置 BLE 设备名称
    pub fn with_ble_name(mut self, name: &str) -> Self {
        let _ = self.ble_device_name.clear();
        let _ = self.ble_device_name.push_str(name);
        self
    }

    /// 设置静态 IP 配置
    pub fn with_static_ip(mut self, ip: [u8; 4], gateway: [u8; 4], dns: [u8; 4]) -> Self {
        self.dhcp_enabled = false;
        self.static_ip = Some(ip);
        self.gateway = Some(gateway);
        self.dns_server = Some(dns);
        self
    }
}

// ===== WiFi 配置常量 =====

/// WiFi 连接超时时间 (毫秒)
pub const WIFI_CONNECT_TIMEOUT_MS: u32 = 30_000;

/// WiFi 扫描超时时间 (毫秒)
pub const WIFI_SCAN_TIMEOUT_MS: u32 = 10_000;

/// WiFi 重连间隔 (毫秒)
pub const WIFI_RECONNECT_INTERVAL_MS: u32 = 5_000;

/// WiFi 最大重连次数
pub const WIFI_MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// WiFi 事件队列大小
pub const WIFI_EVENT_QUEUE_SIZE: usize = 8;

/// WiFi 扫描结果最大数量
pub const WIFI_MAX_SCAN_RESULTS: usize = 16;

// ===== BLE 配置常量 =====

/// BLE 广播间隔 (毫秒) - 快速广播
pub const BLE_ADV_INTERVAL_FAST_MS: u32 = 100;

/// BLE 广播间隔 (毫秒) - 慢速广播 (省电)
pub const BLE_ADV_INTERVAL_SLOW_MS: u32 = 1000;

/// BLE 连接间隔最小值 (1.25ms 单位)
pub const BLE_CONN_INTERVAL_MIN: u16 = 6;  // 7.5ms

/// BLE 连接间隔最大值 (1.25ms 单位)
pub const BLE_CONN_INTERVAL_MAX: u16 = 24; // 30ms

/// BLE 从机延迟
pub const BLE_SLAVE_LATENCY: u16 = 0;

/// BLE 监督超时 (10ms 单位)
pub const BLE_SUPERVISION_TIMEOUT: u16 = 400; // 4 秒

/// BLE MTU 大小
pub const BLE_MTU_SIZE: u16 = 247;

/// BLE 最大连接数
pub const BLE_MAX_CONNECTIONS: usize = 3;

/// BLE 事件队列大小
pub const BLE_EVENT_QUEUE_SIZE: usize = 8;

// ===== TCP/IP 配置常量 =====

/// TCP 接收缓冲区大小
pub const TCP_RX_BUFFER_SIZE: usize = 4096;

/// TCP 发送缓冲区大小
pub const TCP_TX_BUFFER_SIZE: usize = 4096;

/// UDP 接收缓冲区大小
pub const UDP_RX_BUFFER_SIZE: usize = 2048;

/// UDP 发送缓冲区大小
pub const UDP_TX_BUFFER_SIZE: usize = 2048;

/// 最大 TCP Socket 数量
pub const MAX_TCP_SOCKETS: usize = 4;

/// 最大 UDP Socket 数量
pub const MAX_UDP_SOCKETS: usize = 4;

/// DNS 缓存大小
pub const DNS_CACHE_SIZE: usize = 4;

/// DHCP 超时时间 (秒)
pub const DHCP_TIMEOUT_SECS: u32 = 30;

/// TCP 连接超时 (秒)
pub const TCP_CONNECT_TIMEOUT_SECS: u32 = 10;

/// TCP Keep-Alive 间隔 (秒)
pub const TCP_KEEPALIVE_INTERVAL_SECS: u32 = 60;

// ===== 网络缓冲区配置 =====

/// 以太网帧最大大小
pub const ETHERNET_MTU: usize = 1514;

/// IP 包最大大小
pub const IP_MTU: usize = 1500;

/// 网络缓冲区池大小
pub const NET_BUFFER_POOL_SIZE: usize = 16;

/// 单个网络缓冲区大小
pub const NET_BUFFER_SIZE: usize = 1536;
