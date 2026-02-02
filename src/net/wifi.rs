//! WiFi 模块
//!
//! 提供 ESP32-S3 的 WiFi STA 和 AP 模式支持。
//!
//! # 功能
//!
//! - WiFi 网络扫描
//! - STA 模式连接到 AP
//! - AP 模式创建热点
//! - 连接状态监控
//! - 自动重连
//!
//! # 示例
//!
//! ```ignore
//! use rustrtos::net::wifi::{WifiController, WifiMode};
//!
//! let mut controller = WifiController::new(wifi, radio_clk, rng).await?;
//! controller.set_mode(WifiMode::Sta).await?;
//! controller.connect("MySSID", "password").await?;
//!
//! // 等待获取 IP 地址
//! let ip = controller.wait_for_ip().await?;
//! println!("Got IP: {:?}", ip);
//! ```

use core::fmt;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use heapless::{String, Vec};

use super::config::*;

// ===== 错误类型 =====

/// WiFi 错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiError {
    /// 未初始化
    NotInitialized,
    /// 连接失败
    ConnectionFailed,
    /// 认证失败 (密码错误)
    AuthenticationFailed,
    /// 找不到网络
    NetworkNotFound,
    /// 连接超时
    Timeout,
    /// 已断开连接
    Disconnected,
    /// 内部错误
    InternalError,
    /// 配置错误
    ConfigError,
    /// 扫描失败
    ScanFailed,
    /// 资源不足
    OutOfMemory,
    /// 不支持的操作
    Unsupported,
}

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "WiFi not initialized"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::AuthenticationFailed => write!(f, "Authentication failed"),
            Self::NetworkNotFound => write!(f, "Network not found"),
            Self::Timeout => write!(f, "Operation timeout"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::InternalError => write!(f, "Internal error"),
            Self::ConfigError => write!(f, "Configuration error"),
            Self::ScanFailed => write!(f, "Scan failed"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::Unsupported => write!(f, "Unsupported operation"),
        }
    }
}

// ===== WiFi 模式 =====

/// WiFi 工作模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiMode {
    /// 未配置
    #[default]
    None,
    /// Station 模式 (客户端)
    Sta,
    /// Access Point 模式 (热点)
    Ap,
    /// 同时支持 STA 和 AP
    ApSta,
}

// ===== WiFi 事件 =====

/// WiFi 事件类型
#[derive(Debug, Clone)]
pub enum WifiEvent {
    /// 已连接到 AP
    StaConnected,
    /// 已从 AP 断开
    StaDisconnected {
        /// 断开原因
        reason: DisconnectReason,
    },
    /// 获取到 IP 地址
    GotIp {
        /// IP 地址
        ip: [u8; 4],
        /// 网关
        gateway: [u8; 4],
        /// 子网掩码
        netmask: [u8; 4],
    },
    /// 扫描完成
    ScanDone {
        /// 找到的网络数量
        count: usize,
    },
    /// AP 模式: 客户端连接
    ApStaConnected {
        /// 客户端 MAC 地址
        mac: [u8; 6],
    },
    /// AP 模式: 客户端断开
    ApStaDisconnected {
        /// 客户端 MAC 地址
        mac: [u8; 6],
    },
}

/// 断开连接原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// 未指定
    Unspecified,
    /// 认证过期
    AuthExpired,
    /// 认证离开
    AuthLeave,
    /// 关联过期
    AssocExpired,
    /// 关联数量过多
    AssocTooMany,
    /// 未认证
    NotAuthenticated,
    /// 未关联
    NotAssociated,
    /// 已离开
    AssocLeave,
    /// 关联未认证
    AssocNotAuth,
    /// 信道错误
    BadChannel,
    /// 信号弱
    BeaconTimeout,
    /// AP 未找到
    NoApFound,
    /// 密码错误
    WrongPassword,
    /// 连接失败
    ConnectionFail,
    /// AP 握手超时
    ApHandshakeFail,
}

impl Default for DisconnectReason {
    fn default() -> Self {
        Self::Unspecified
    }
}

// ===== 扫描结果 =====

/// WiFi 扫描结果
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// SSID
    pub ssid: String<32>,
    /// BSSID (MAC 地址)
    pub bssid: [u8; 6],
    /// 信号强度 (dBm)
    pub rssi: i8,
    /// 信道
    pub channel: u8,
    /// 安全类型
    pub auth_mode: AuthMode,
}

/// WiFi 安全模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// 开放网络
    #[default]
    Open,
    /// WEP
    Wep,
    /// WPA-PSK
    WpaPsk,
    /// WPA2-PSK
    Wpa2Psk,
    /// WPA/WPA2-PSK
    WpaWpa2Psk,
    /// WPA3-PSK
    Wpa3Psk,
    /// WPA2/WPA3-PSK
    Wpa2Wpa3Psk,
    /// 企业级
    Enterprise,
}

// ===== WiFi 状态 =====

/// WiFi 连接状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiState {
    /// 未初始化
    #[default]
    Uninitialized,
    /// 已初始化但未连接
    Idle,
    /// 正在扫描
    Scanning,
    /// 正在连接
    Connecting,
    /// 已连接
    Connected,
    /// 正在获取 IP
    GettingIp,
    /// 已获取 IP
    Ready,
    /// 已断开
    Disconnected,
}

// ===== WiFi 控制器 =====

/// WiFi 控制器
///
/// 管理 WiFi 连接生命周期，提供异步 API。
pub struct WifiController<'a> {
    /// 当前模式
    mode: WifiMode,
    /// 当前状态
    state: WifiState,
    /// 当前 SSID
    ssid: String<32>,
    /// 当前密码
    password: String<64>,
    /// IP 地址
    ip_address: Option<[u8; 4]>,
    /// 网关地址
    gateway: Option<[u8; 4]>,
    /// 事件通道
    event_channel: &'a Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    /// 连接信号
    connected_signal: &'a Signal<CriticalSectionRawMutex, bool>,
    /// 扫描结果
    scan_results: Vec<ScanResult, WIFI_MAX_SCAN_RESULTS>,
    /// 重连计数
    reconnect_count: u32,
    /// 自动重连启用
    auto_reconnect: bool,
}

impl<'a> WifiController<'a> {
    /// 创建新的 WiFi 控制器
    ///
    /// # 注意
    ///
    /// 此函数需要在系统初始化时调用，传入所需的外设和静态分配的通道。
    pub fn new(
        event_channel: &'a Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
        connected_signal: &'a Signal<CriticalSectionRawMutex, bool>,
    ) -> Self {
        Self {
            mode: WifiMode::None,
            state: WifiState::Uninitialized,
            ssid: String::new(),
            password: String::new(),
            ip_address: None,
            gateway: None,
            event_channel,
            connected_signal,
            scan_results: Vec::new(),
            reconnect_count: 0,
            auto_reconnect: true,
        }
    }

    /// 初始化 WiFi 硬件
    ///
    /// 注意：在调用此函数之前，必须先初始化 esp-radio:
    /// ```ignore
    /// let timg0 = TimerGroup::new(peripherals.TIMG0);
    /// esp_rtos::start(timg0.timer0);
    /// let _controller = esp_radio::init().unwrap();
    /// ```
    pub async fn init(&mut self) -> Result<(), WifiError> {
        // esp-radio 的初始化在更高层完成
        // 这里只是设置本地状态
        self.state = WifiState::Idle;
        Ok(())
    }

    /// 设置 WiFi 模式
    ///
    /// **注意**: 这只更新内部状态。实际的 WiFi 模式配置应通过 esp-radio 的
    /// `WifiController::set_config()` 完成。参见 `examples/wifi_connect.rs`。
    pub async fn set_mode(&mut self, mode: WifiMode) -> Result<(), WifiError> {
        if self.state == WifiState::Uninitialized {
            return Err(WifiError::NotInitialized);
        }

        self.mode = mode;
        // 状态管理层 - 实际模式设置通过 esp_radio::wifi::WifiController 完成
        Ok(())
    }

    /// 获取当前模式
    pub fn mode(&self) -> WifiMode {
        self.mode
    }

    /// 获取当前状态
    pub fn state(&self) -> WifiState {
        self.state
    }

    /// 扫描周围的 WiFi 网络
    ///
    /// **注意**: 此函数仅管理状态。实际扫描操作应通过 esp-radio API 完成。
    /// 请参考 `examples/wifi_scan.rs`。
    pub async fn scan(&mut self) -> Result<&[ScanResult], WifiError> {
        if self.state == WifiState::Uninitialized {
            return Err(WifiError::NotInitialized);
        }

        self.state = WifiState::Scanning;
        self.scan_results.clear();

        // 状态管理层 - 实际扫描通过 esp_radio::wifi::WifiController 完成
        // 等待外部扫描完成的延迟
        Timer::after(Duration::from_millis(100)).await;

        self.state = WifiState::Idle;
        
        // 发送扫描完成事件
        let _ = self.event_channel.try_send(WifiEvent::ScanDone {
            count: self.scan_results.len(),
        });

        Ok(&self.scan_results)
    }

    /// 连接到指定的 WiFi 网络
    ///
    /// # 参数
    ///
    /// - `ssid` - 网络名称
    /// - `password` - 密码 (开放网络传空字符串)
    pub async fn connect(&mut self, ssid: &str, password: &str) -> Result<(), WifiError> {
        if self.state == WifiState::Uninitialized {
            return Err(WifiError::NotInitialized);
        }

        // 保存凭据
        self.ssid.clear();
        let _ = self.ssid.push_str(ssid);
        self.password.clear();
        let _ = self.password.push_str(password);

        self.state = WifiState::Connecting;
        self.reconnect_count = 0;

        // 状态管理层 - 实际连接通过 esp_radio::wifi::WifiController::connect_async() 完成
        // 这里等待外部控制器触发的连接信号
        let timeout = Duration::from_millis(WIFI_CONNECT_TIMEOUT_MS as u64);
        
        match embassy_time::with_timeout(timeout, self.wait_connected()).await {
            Ok(result) => result,
            Err(_) => {
                self.state = WifiState::Disconnected;
                Err(WifiError::Timeout)
            }
        }
    }

    /// 等待连接建立
    async fn wait_connected(&mut self) -> Result<(), WifiError> {
        // 等待连接信号
        loop {
            if self.connected_signal.wait().await {
                self.state = WifiState::Connected;
                
                // 发送连接事件
                let _ = self.event_channel.try_send(WifiEvent::StaConnected);
                
                return Ok(());
            } else {
                return Err(WifiError::ConnectionFailed);
            }
        }
    }

    /// 断开 WiFi 连接
    ///
    /// **注意**: 此函数仅更新内部状态。实际断开操作应通过
    /// `esp_radio::wifi::WifiController::disconnect_async()` 完成。
    pub async fn disconnect(&mut self) -> Result<(), WifiError> {
        if self.state == WifiState::Uninitialized {
            return Err(WifiError::NotInitialized);
        }

        // 状态管理层 - 实际断开通过 esp_radio::wifi::WifiController 完成
        self.state = WifiState::Disconnected;
        self.ip_address = None;
        self.gateway = None;

        let _ = self.event_channel.try_send(WifiEvent::StaDisconnected {
            reason: DisconnectReason::AssocLeave,
        });

        Ok(())
    }

    /// 等待获取 IP 地址
    ///
    /// **注意**: IP 地址获取应通过 embassy-net 的 DHCP 客户端完成。
    /// 此函数仅等待 `set_ip_address()` 被调用。参见 `examples/tcp_client.rs`。
    pub async fn wait_for_ip(&mut self) -> Result<[u8; 4], WifiError> {
        if self.state != WifiState::Connected && self.state != WifiState::GettingIp {
            return Err(WifiError::NotInitialized);
        }

        self.state = WifiState::GettingIp;

        // 等待外部设置 IP 地址 (通过 set_ip_address 方法)
        // DHCP 客户端应通过 embassy-net::DhcpConfig 配置
        let timeout = Duration::from_secs(DHCP_TIMEOUT_SECS as u64);
        
        match embassy_time::with_timeout(timeout, self.wait_ip_internal()).await {
            Ok(ip) => {
                self.state = WifiState::Ready;
                Ok(ip)
            }
            Err(_) => Err(WifiError::Timeout),
        }
    }

    /// 内部等待 IP
    async fn wait_ip_internal(&self) -> [u8; 4] {
        // 轮询等待 IP 地址被设置
        // 应用层应通过 set_ip_address() 设置
        loop {
            if let Some(ip) = self.ip_address {
                return ip;
            }
            Timer::after(Duration::from_millis(100)).await;
        }
    }

    /// 获取当前 IP 地址
    pub fn ip_address(&self) -> Option<[u8; 4]> {
        self.ip_address
    }

    /// 获取网关地址
    pub fn gateway(&self) -> Option<[u8; 4]> {
        self.gateway
    }

    /// 设置 IP 地址 (由外部 DHCP 客户端调用)
    ///
    /// 当使用 embassy-net 获取到 IP 地址后，调用此方法更新状态。
    pub fn set_ip_address(&mut self, ip: [u8; 4], gateway: [u8; 4]) {
        self.ip_address = Some(ip);
        self.gateway = Some(gateway);
        self.state = WifiState::Ready;
        
        let _ = self.event_channel.try_send(WifiEvent::GotIp {
            ip,
            gateway,
            netmask: [255, 255, 255, 0], // 默认子网掩码
        });
    }

    /// 设置连接状态 (由外部控制器回调调用)
    pub fn set_connected(&mut self, connected: bool) {
        if connected {
            self.state = WifiState::Connected;
            let _ = self.event_channel.try_send(WifiEvent::StaConnected);
        } else {
            self.state = WifiState::Disconnected;
            self.ip_address = None;
            self.gateway = None;
        }
        self.connected_signal.signal(connected);
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        matches!(self.state, WifiState::Connected | WifiState::GettingIp | WifiState::Ready)
    }

    /// 启用/禁用自动重连
    pub fn set_auto_reconnect(&mut self, enabled: bool) {
        self.auto_reconnect = enabled;
    }

    /// 获取扫描结果
    pub fn scan_results(&self) -> &[ScanResult] {
        &self.scan_results
    }

    /// 接收 WiFi 事件
    pub async fn recv_event(&self) -> WifiEvent {
        self.event_channel.receive().await
    }

    /// 尝试接收 WiFi 事件 (非阻塞)
    pub fn try_recv_event(&self) -> Option<WifiEvent> {
        self.event_channel.try_receive().ok()
    }
}

// ===== AP 模式配置 =====

/// AP 模式配置
#[derive(Debug, Clone)]
pub struct ApConfig {
    /// SSID
    pub ssid: String<32>,
    /// 密码 (空字符串表示开放网络)
    pub password: String<64>,
    /// 信道 (1-13)
    pub channel: u8,
    /// 最大客户端数量
    pub max_clients: u8,
    /// 隐藏 SSID
    pub hidden: bool,
}

impl Default for ApConfig {
    fn default() -> Self {
        Self {
            ssid: String::try_from("RustRTOS-AP").unwrap_or_default(),
            password: String::new(),
            channel: 1,
            max_clients: 4,
            hidden: false,
        }
    }
}

// ===== WiFi 统计信息 =====

/// WiFi 统计信息
#[derive(Debug, Clone, Default)]
pub struct WifiStats {
    /// 发送的数据包数量
    pub tx_packets: u32,
    /// 接收的数据包数量
    pub rx_packets: u32,
    /// 发送的字节数
    pub tx_bytes: u64,
    /// 接收的字节数
    pub rx_bytes: u64,
    /// 发送错误数
    pub tx_errors: u32,
    /// 接收错误数
    pub rx_errors: u32,
    /// 当前 RSSI (dBm)
    pub rssi: i8,
    /// 连接时长 (秒)
    pub connected_time: u32,
}
