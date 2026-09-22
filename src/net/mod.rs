#![allow(unused_imports)]

pub mod config;

#[cfg(feature = "wifi")]
pub mod wifi;

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub mod ble;

#[cfg(feature = "network")]
pub mod tcp;

#[cfg(feature = "wifi")]
pub use wifi::{WifiController, WifiMode, WifiEvent, WifiError, ScanResult};

#[cfg(feature = "ble")]
pub use ble::{BleController, BleEvent, BleError, BleState, AdvertiseConfig, ConnectionInfo, DisconnectReason};

#[cfg(feature = "network")]
pub use tcp::{TcpClient, TcpServer, UdpSocket, NetworkStack, NetworkError};

pub use config::NetworkConfig;
