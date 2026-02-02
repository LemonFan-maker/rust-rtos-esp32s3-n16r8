//! BLE (低功耗蓝牙) 模块
//!
//! 提供 ESP32-S3 的 BLE 功能支持，默认使用 trouble-host 协议栈，
//! 也可通过 `ble-esp` feature 使用 esp-wifi 内置的 BLE 实现。
//!
//! # 功能
//!
//! - BLE 广播 (Advertising)
//! - GATT Server (外设角色)
//! - GATT Client (中心角色)
//! - 连接管理
//! - 安全配对 (可选)
//!
//! # 示例
//!
//! ```ignore
//! use rustrtos::net::ble::{BleController, AdvertiseConfig};
//!
//! let mut controller = BleController::new(bt, radio_clk).await?;
//!
//! // 配置广播
//! let config = AdvertiseConfig::default()
//!     .with_name("RustRTOS")
//!     .with_interval_ms(100);
//!
//! controller.start_advertising(config).await?;
//! ```

use core::fmt;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use heapless::{String, Vec};

use super::config::*;

// ===== 错误类型 =====

/// BLE 错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleError {
    /// 未初始化
    NotInitialized,
    /// 已在广播中
    AlreadyAdvertising,
    /// 广播启动失败
    AdvertisingFailed,
    /// 连接失败
    ConnectionFailed,
    /// 连接已断开
    Disconnected,
    /// 配对失败
    PairingFailed,
    /// GATT 操作失败
    GattError,
    /// 资源不足
    OutOfMemory,
    /// 无效参数
    InvalidParameter,
    /// 操作超时
    Timeout,
    /// 内部错误
    InternalError,
    /// 不支持的操作
    Unsupported,
    /// 已达最大连接数
    MaxConnectionsReached,
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "BLE not initialized"),
            Self::AlreadyAdvertising => write!(f, "Already advertising"),
            Self::AdvertisingFailed => write!(f, "Advertising failed"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::PairingFailed => write!(f, "Pairing failed"),
            Self::GattError => write!(f, "GATT error"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::InvalidParameter => write!(f, "Invalid parameter"),
            Self::Timeout => write!(f, "Timeout"),
            Self::InternalError => write!(f, "Internal error"),
            Self::Unsupported => write!(f, "Unsupported"),
            Self::MaxConnectionsReached => write!(f, "Max connections reached"),
        }
    }
}

// ===== BLE 事件 =====

/// BLE 事件类型
#[derive(Debug, Clone)]
pub enum BleEvent {
    /// 广播已开始
    AdvertisingStarted,
    /// 广播已停止
    AdvertisingStopped,
    /// 设备已连接
    Connected {
        /// 连接句柄
        conn_handle: u16,
        /// 对端地址
        peer_addr: [u8; 6],
    },
    /// 设备已断开
    Disconnected {
        /// 连接句柄
        conn_handle: u16,
        /// 断开原因
        reason: DisconnectReason,
    },
    /// MTU 已更新
    MtuUpdated {
        /// 连接句柄
        conn_handle: u16,
        /// 新的 MTU 值
        mtu: u16,
    },
    /// 收到写请求
    WriteRequest {
        /// 连接句柄
        conn_handle: u16,
        /// 属性句柄
        attr_handle: u16,
        /// 数据长度
        len: usize,
    },
    /// 收到读请求
    ReadRequest {
        /// 连接句柄
        conn_handle: u16,
        /// 属性句柄
        attr_handle: u16,
    },
    /// 通知已发送
    NotificationSent {
        /// 连接句柄
        conn_handle: u16,
    },
    /// 配对完成
    PairingComplete {
        /// 连接句柄
        conn_handle: u16,
        /// 是否绑定
        bonded: bool,
    },
}

/// BLE 断开原因
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisconnectReason {
    /// 未知原因
    #[default]
    Unknown,
    /// 远端用户断开
    RemoteUserTerminated,
    /// 本地用户断开
    LocalHostTerminated,
    /// 连接超时
    ConnectionTimeout,
    /// 认证失败
    AuthenticationFailure,
    /// 连接参数不接受
    UnacceptableConnectionParameters,
}

// ===== BLE 状态 =====

/// BLE 状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BleState {
    /// 未初始化
    #[default]
    Uninitialized,
    /// 空闲
    Idle,
    /// 正在广播
    Advertising,
    /// 已连接
    Connected,
    /// 正在扫描 (中心角色)
    Scanning,
}

// ===== 广播配置 =====

/// 广播配置
#[derive(Debug, Clone)]
pub struct AdvertiseConfig {
    /// 设备名称
    pub name: String<32>,
    /// 广播间隔 (毫秒)
    pub interval_ms: u32,
    /// 是否可连接
    pub connectable: bool,
    /// 是否可扫描
    pub scannable: bool,
    /// 广播数据 (最多 31 字节)
    pub adv_data: Vec<u8, 31>,
    /// 扫描响应数据 (最多 31 字节)
    pub scan_rsp_data: Vec<u8, 31>,
    /// 广播超时 (0 = 无限)
    pub timeout_secs: u16,
}

impl Default for AdvertiseConfig {
    fn default() -> Self {
        Self {
            name: String::try_from("RustRTOS").unwrap_or_default(),
            interval_ms: BLE_ADV_INTERVAL_FAST_MS,
            connectable: true,
            scannable: true,
            adv_data: Vec::new(),
            scan_rsp_data: Vec::new(),
            timeout_secs: 0,
        }
    }
}

impl AdvertiseConfig {
    /// 设置设备名称
    pub fn with_name(mut self, name: &str) -> Self {
        self.name.clear();
        let _ = self.name.push_str(name);
        self
    }

    /// 设置广播间隔
    pub fn with_interval_ms(mut self, interval: u32) -> Self {
        self.interval_ms = interval;
        self
    }

    /// 设置是否可连接
    pub fn with_connectable(mut self, connectable: bool) -> Self {
        self.connectable = connectable;
        self
    }

    /// 设置广播超时
    pub fn with_timeout_secs(mut self, timeout: u16) -> Self {
        self.timeout_secs = timeout;
        self
    }

    /// 添加自定义广播数据
    pub fn with_adv_data(mut self, data: &[u8]) -> Self {
        self.adv_data.clear();
        let _ = self.adv_data.extend_from_slice(data);
        self
    }
}

// ===== 连接信息 =====

/// BLE 连接信息
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    /// 连接句柄
    pub handle: u16,
    /// 对端地址
    pub peer_addr: [u8; 6],
    /// 连接间隔 (1.25ms 单位)
    pub interval: u16,
    /// 从机延迟
    pub latency: u16,
    /// 监督超时 (10ms 单位)
    pub timeout: u16,
    /// 当前 MTU
    pub mtu: u16,
    /// 是否已绑定
    pub bonded: bool,
}

// ===== GATT 服务定义 =====

/// GATT 服务 UUID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uuid {
    /// 16 位 UUID
    Uuid16(u16),
    /// 128 位 UUID
    Uuid128([u8; 16]),
}

impl Uuid {
    /// 创建 16 位 UUID
    pub const fn from_u16(uuid: u16) -> Self {
        Self::Uuid16(uuid)
    }

    /// 创建 128 位 UUID
    pub const fn from_u128(uuid: [u8; 16]) -> Self {
        Self::Uuid128(uuid)
    }
}

/// GATT 特征属性
#[derive(Debug, Clone, Copy)]
pub struct CharacteristicProps {
    /// 可读
    pub read: bool,
    /// 可写 (无响应)
    pub write_without_response: bool,
    /// 可写 (有响应)
    pub write: bool,
    /// 通知
    pub notify: bool,
    /// 指示
    pub indicate: bool,
}

impl Default for CharacteristicProps {
    fn default() -> Self {
        Self {
            read: true,
            write_without_response: false,
            write: false,
            notify: false,
            indicate: false,
        }
    }
}

impl CharacteristicProps {
    /// 可读可写
    pub const fn read_write() -> Self {
        Self {
            read: true,
            write_without_response: false,
            write: true,
            notify: false,
            indicate: false,
        }
    }

    /// 可读可通知
    pub const fn read_notify() -> Self {
        Self {
            read: true,
            write_without_response: false,
            write: false,
            notify: true,
            indicate: false,
        }
    }
}

/// GATT 特征定义
#[derive(Debug, Clone)]
pub struct Characteristic {
    /// UUID
    pub uuid: Uuid,
    /// 属性
    pub props: CharacteristicProps,
    /// 属性句柄 (由协议栈分配)
    pub handle: u16,
    /// 值句柄 (由协议栈分配)
    pub value_handle: u16,
}

/// GATT 服务定义
#[derive(Debug, Clone)]
pub struct Service {
    /// 服务 UUID
    pub uuid: Uuid,
    /// 是否为主要服务
    pub primary: bool,
    /// 服务句柄 (由协议栈分配)
    pub handle: u16,
    /// 特征数量
    pub characteristic_count: usize,
}

// ===== BLE 控制器 =====

/// BLE 控制器
///
/// 管理 BLE 连接和 GATT 服务。
pub struct BleController<'a> {
    /// 当前状态
    state: BleState,
    /// 事件通道
    event_channel: &'a Channel<CriticalSectionRawMutex, BleEvent, BLE_EVENT_QUEUE_SIZE>,
    /// 连接信号
    connected_signal: &'a Signal<CriticalSectionRawMutex, bool>,
    /// 活动连接
    connections: Vec<ConnectionInfo, BLE_MAX_CONNECTIONS>,
    /// 本地地址
    local_addr: [u8; 6],
    /// 广播配置
    adv_config: Option<AdvertiseConfig>,
}

impl<'a> BleController<'a> {
    /// 创建新的 BLE 控制器
    pub fn new(
        event_channel: &'a Channel<CriticalSectionRawMutex, BleEvent, BLE_EVENT_QUEUE_SIZE>,
        connected_signal: &'a Signal<CriticalSectionRawMutex, bool>,
    ) -> Self {
        Self {
            state: BleState::Uninitialized,
            event_channel,
            connected_signal,
            connections: Vec::new(),
            local_addr: [0; 6],
            adv_config: None,
        }
    }

    /// 初始化 BLE 硬件
    ///
    /// 注意：在调用此函数之前，必须先初始化 esp-radio:
    /// ```ignore
    /// let timg0 = TimerGroup::new(peripherals.TIMG0);
    /// esp_rtos::start(timg0.timer0);
    /// let _controller = esp_radio::init().unwrap();
    /// ```
    pub async fn init(&mut self) -> Result<(), BleError> {
        // esp-radio 的初始化在更高层完成
        // 这里只是设置本地状态
        self.state = BleState::Idle;
        
        // 生成随机本地地址 (实际应从芯片获取)
        self.local_addr = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        
        Ok(())
    }

    /// 获取当前状态
    pub fn state(&self) -> BleState {
        self.state
    }

    /// 获取本地蓝牙地址
    pub fn local_address(&self) -> [u8; 6] {
        self.local_addr
    }

    /// 开始广播
    ///
    /// **注意**: 此函数仅管理状态。实际广播应通过 trouble-host 的
    /// `Peripheral::advertise()` 完成。参见 `examples/ble_advertise.rs`。
    pub async fn start_advertising(&mut self, config: AdvertiseConfig) -> Result<(), BleError> {
        if self.state == BleState::Uninitialized {
            return Err(BleError::NotInitialized);
        }

        if self.state == BleState::Advertising {
            return Err(BleError::AlreadyAdvertising);
        }

        self.adv_config = Some(config);
        self.state = BleState::Advertising;

        // 状态管理层 - 实际广播通过 trouble_host::Peripheral 完成
        let _ = self.event_channel.try_send(BleEvent::AdvertisingStarted);

        Ok(())
    }

    /// 停止广播
    ///
    /// **注意**: 此函数仅管理状态。实际停止应通过取消 trouble-host 的
    /// advertise future 完成。
    pub async fn stop_advertising(&mut self) -> Result<(), BleError> {
        if self.state != BleState::Advertising {
            return Ok(());
        }

        // 状态管理层 - 停止广播通过取消 future 完成
        self.state = BleState::Idle;
        let _ = self.event_channel.try_send(BleEvent::AdvertisingStopped);

        Ok(())
    }

    /// 断开指定连接
    pub async fn disconnect(&mut self, conn_handle: u16) -> Result<(), BleError> {
        // 查找并移除连接
        if let Some(pos) = self.connections.iter().position(|c| c.handle == conn_handle) {
            let conn = self.connections.remove(pos);
            
            let _ = self.event_channel.try_send(BleEvent::Disconnected {
                conn_handle,
                reason: DisconnectReason::LocalHostTerminated,
            });

            if self.connections.is_empty() {
                self.state = BleState::Idle;
            }
        }

        Ok(())
    }

    /// 断开所有连接
    pub async fn disconnect_all(&mut self) -> Result<(), BleError> {
        while let Some(conn) = self.connections.pop() {
            let _ = self.event_channel.try_send(BleEvent::Disconnected {
                conn_handle: conn.handle,
                reason: DisconnectReason::LocalHostTerminated,
            });
        }
        self.state = BleState::Idle;
        Ok(())
    }

    /// 获取活动连接数
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// 获取连接信息
    pub fn connection_info(&self, handle: u16) -> Option<&ConnectionInfo> {
        self.connections.iter().find(|c| c.handle == handle)
    }

    /// 发送通知
    ///
    /// **注意**: 此函数仅记录状态。实际通知应通过 trouble-host 的
    /// `Characteristic::notify()` 完成。参见 `examples/ble_gatt_server.rs`。
    pub async fn notify(
        &self,
        conn_handle: u16,
        attr_handle: u16,
        data: &[u8],
    ) -> Result<(), BleError> {
        if !self.connections.iter().any(|c| c.handle == conn_handle) {
            return Err(BleError::Disconnected);
        }

        // 状态管理层 - 实际通知通过 trouble_host GATT API 完成
        let _ = attr_handle; // 暂用于类型检查
        let _ = data;
        let _ = self.event_channel.try_send(BleEvent::NotificationSent { conn_handle });

        Ok(())
    }

    /// 接收 BLE 事件
    pub async fn recv_event(&self) -> BleEvent {
        self.event_channel.receive().await
    }

    /// 尝试接收 BLE 事件 (非阻塞)
    pub fn try_recv_event(&self) -> Option<BleEvent> {
        self.event_channel.try_receive().ok()
    }

    /// 等待连接
    pub async fn wait_for_connection(&mut self) -> Result<ConnectionInfo, BleError> {
        loop {
            match self.recv_event().await {
                BleEvent::Connected { conn_handle, peer_addr } => {
                    let conn = ConnectionInfo {
                        handle: conn_handle,
                        peer_addr,
                        interval: BLE_CONN_INTERVAL_MIN,
                        latency: BLE_SLAVE_LATENCY,
                        timeout: BLE_SUPERVISION_TIMEOUT,
                        mtu: 23, // 默认 MTU
                        bonded: false,
                    };
                    
                    if self.connections.push(conn.clone()).is_err() {
                        return Err(BleError::MaxConnectionsReached);
                    }
                    
                    self.state = BleState::Connected;
                    return Ok(conn);
                }
                _ => continue,
            }
        }
    }
}

// ===== GATT Server =====

/// GATT Server 构建器
pub struct GattServerBuilder {
    services: Vec<Service, 8>,
}

impl GattServerBuilder {
    /// 创建新的 GATT Server 构建器
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
        }
    }

    /// 添加服务
    pub fn add_service(mut self, uuid: Uuid, primary: bool) -> Self {
        let service = Service {
            uuid,
            primary,
            handle: 0,
            characteristic_count: 0,
        };
        let _ = self.services.push(service);
        self
    }

    /// 构建 GATT Server
    pub fn build(self) -> GattServer {
        GattServer {
            services: self.services,
        }
    }
}

impl Default for GattServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// GATT Server
pub struct GattServer {
    services: Vec<Service, 8>,
}

impl GattServer {
    /// 获取服务列表
    pub fn services(&self) -> &[Service] {
        &self.services
    }

    /// 注册到 BLE 控制器
    ///
    /// **注意**: 此函数为占位实现。trouble-host 的 GATT Server 应通过
    /// `#[gatt_server]` 宏定义，然后通过 `GattConnection::with_attribute_server()` 注册。
    /// 参见 `examples/ble_gatt_server.rs`。
    pub async fn register(&self, _controller: &mut BleController<'_>) -> Result<(), BleError> {
        // 状态管理层 - 实际注册通过 trouble_host GATT 宏完成
        Ok(())
    }
}

// ===== BLE 统计信息 =====

/// BLE 统计信息
#[derive(Debug, Clone, Default)]
pub struct BleStats {
    /// 广播包发送数量
    pub adv_packets_sent: u32,
    /// 连接次数
    pub connections_total: u32,
    /// 当前活动连接数
    pub connections_active: u32,
    /// 发送的数据包
    pub tx_packets: u32,
    /// 接收的数据包
    pub rx_packets: u32,
    /// 发送错误
    pub tx_errors: u32,
    /// 接收错误
    pub rx_errors: u32,
}
