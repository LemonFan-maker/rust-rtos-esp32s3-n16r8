//! 网络协议栈模块
//! 提供WiFi和BLE网络功能支持:
//! - WiFi STA/AP模式连接管理
//! - TCP/UDP Socket通信(基于smoltcp + embassy-net)
//! - BLE广播和GATT服务(基于trouble-host或esp-wifi/ble)
//! Features
//! - wifi - 启用WiFi功能
//! - ble - 启用BLE功能(使用trouble-host)
//! - ble-esp - 启用BLE功能(使用esp-wifi内置)
//! - network - 启用完整TCP/IP网络栈
//! - coex - WiFi + BLE共存模式
//! 示例
//! use rustrtos::net::{wifi, ble};
//! // WiFi连接
//! let mut wifi_controller = wifi::WifiController::new(peripherals.WIFI);
//! wifi_controller.connect("SSID", "password").await?;
//! // BLE广播
//! let ble_controller = ble::BleController::new(peripherals.BT);
//! ble_controller.start_advertising().await?;

#![allow(unused_imports)]

pub mod config;

#[cfg(feature = "wifi")]
pub mod wifi;

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub mod ble;

#[cfg(feature = "network")]
pub mod tcp;

// 公共类型重导出

#[cfg(feature = "wifi")]
pub use wifi::{WifiController, WifiMode, WifiEvent, WifiError, ScanResult};

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub use ble::{BleController, BleEvent, BleError, AdvertiseConfig};

#[cfg(feature = "network")]
pub use tcp::{TcpClient, TcpServer, UdpSocket, NetworkStack, NetworkError};

pub use config::NetworkConfig;

// 网络初始化函数

use esp_hal::peripherals::Peripherals;

/// 网络初始化结果
#[cfg(feature = "wifi")]
pub struct NetworkResources<'a> {
    /// WiFi控制器
    pub wifi: WifiController<'a>,
    /// 网络栈(如果启用了network feature)
    #[cfg(feature = "network")]
    pub stack: NetworkStack<'a>,
}

/// 初始化网络子系统
/// 此函数应在系统启动时调用一次，用于初始化WiFi和/或BLE控制器。
/// 注意：在调用此函数之前，必须先完成以下初始化：
/// 1. TimerGroup初始化
/// 2. esp_rtos::start()调用
/// 3. esp_radio::init()调用
/// 返回
/// 返回初始化后的网络资源结构
#[cfg(feature = "wifi")]
pub async fn init_wifi() -> Result<(), WifiError> {
    // WiFi初始化将在wifi模块中实现
    // esp-radio的init()函数会自动获取所需的外设
    Ok(())
}

/// 初始化BLE子系统
/// 注意: 此函数已废弃。BLE应直接通过esp-radio和trouble-host初始化。
/// 请参考examples/ble_advertise.rs和examples/ble_gatt_server.rs。
/// Example
/// // 推荐的直接初始化方式:
/// let radio_controller = esp_radio::init().unwrap();
/// let connector = esp_radio::ble::controller::BleConnector::new(
///     &radio_controller, peripherals.BT, Default::default()
/// ).unwrap();
/// let controller: ExternalController<_, 20> = ExternalController::new(connector);
#[cfg(any(feature = "ble", feature = "ble-esp"))]
#[deprecated(since = "0.2.0", note = "Use esp-radio directly. See examples/ble_advertise.rs")]
pub async fn init_ble() -> Result<BleController<'static>, BleError> {
    // BLE初始化应在应用层通过esp-radio + trouble-host完成
    // 此函数保留仅为API兼容性
    Err(BleError::Unsupported)
}
