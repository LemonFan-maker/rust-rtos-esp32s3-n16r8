//! 网络协议栈模块
//!
//! 提供 WiFi 和 BLE 网络功能支持:
//! - WiFi STA/AP 模式连接管理
//! - TCP/UDP Socket 通信 (基于 smoltcp + embassy-net)
//! - BLE 广播和 GATT 服务 (基于 trouble-host 或 esp-wifi/ble)
//!
//! # Features
//!
//! - `wifi` - 启用 WiFi 功能
//! - `ble` - 启用 BLE 功能 (使用 trouble-host)
//! - `ble-esp` - 启用 BLE 功能 (使用 esp-wifi 内置)
//! - `network` - 启用完整 TCP/IP 网络栈
//! - `coex` - WiFi + BLE 共存模式
//!
//! # 示例
//!
//! ```ignore
//! use rustrtos::net::{wifi, ble};
//!
//! // WiFi 连接
//! let mut wifi_controller = wifi::WifiController::new(peripherals.WIFI);
//! wifi_controller.connect("SSID", "password").await?;
//!
//! // BLE 广播
//! let ble_controller = ble::BleController::new(peripherals.BT);
//! ble_controller.start_advertising().await?;
//! ```

#![allow(unused_imports)]

pub mod config;

#[cfg(feature = "wifi")]
pub mod wifi;

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub mod ble;

#[cfg(feature = "network")]
pub mod tcp;

// ===== 公共类型重导出 =====

#[cfg(feature = "wifi")]
pub use wifi::{WifiController, WifiMode, WifiEvent, WifiError, ScanResult};

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub use ble::{BleController, BleEvent, BleError, AdvertiseConfig};

#[cfg(feature = "network")]
pub use tcp::{TcpClient, TcpServer, UdpSocket, NetworkStack, NetworkError};

pub use config::NetworkConfig;

// ===== 网络初始化函数 =====

use esp_hal::peripherals::Peripherals;

/// 网络初始化结果
#[cfg(feature = "wifi")]
pub struct NetworkResources<'a> {
    /// WiFi 控制器
    pub wifi: WifiController<'a>,
    /// 网络栈 (如果启用了 network feature)
    #[cfg(feature = "network")]
    pub stack: NetworkStack<'a>,
}

/// 初始化网络子系统
///
/// 此函数应在系统启动时调用一次，用于初始化 WiFi 和/或 BLE 控制器。
///
/// 注意：在调用此函数之前，必须先完成以下初始化：
/// 1. TimerGroup 初始化
/// 2. esp_rtos::start() 调用
/// 3. esp_radio::init() 调用
///
/// # 返回
///
/// 返回初始化后的网络资源结构
#[cfg(feature = "wifi")]
pub async fn init_wifi() -> Result<(), WifiError> {
    // WiFi 初始化将在 wifi 模块中实现
    // esp-radio 的 init() 函数会自动获取所需的外设
    Ok(())
}

/// 初始化 BLE 子系统
///
/// **注意**: 此函数已废弃。BLE 应直接通过 esp-radio 和 trouble-host 初始化。
/// 请参考 `examples/ble_advertise.rs` 和 `examples/ble_gatt_server.rs`。
///
/// # Example
/// ```ignore
/// // 推荐的直接初始化方式:
/// let radio_controller = esp_radio::init().unwrap();
/// let connector = esp_radio::ble::controller::BleConnector::new(
///     &radio_controller, peripherals.BT, Default::default()
/// ).unwrap();
/// let controller: ExternalController<_, 20> = ExternalController::new(connector);
/// ```
#[cfg(any(feature = "ble", feature = "ble-esp"))]
#[deprecated(since = "0.2.0", note = "Use esp-radio directly. See examples/ble_advertise.rs")]
pub async fn init_ble() -> Result<BleController<'static>, BleError> {
    // BLE 初始化应在应用层通过 esp-radio + trouble-host 完成
    // 此函数保留仅为 API 兼容性
    Err(BleError::Unsupported)
}
