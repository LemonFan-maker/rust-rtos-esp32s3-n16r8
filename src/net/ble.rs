use core::fmt;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use heapless::{String, Vec};

#[cfg(feature = "ble")]
use trouble_host::prelude::*;

use esp_radio::ble::controller::BleConnector;

use super::config::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleError {
    NotInitialized,
    AlreadyAdvertising,
    NotAdvertising,
    AdvertisingFailed,
    ConnectionFailed,
    Disconnected,
    OutOfMemory,
    InvalidParameter,
    Timeout,
    InternalError,
    MaxConnectionsReached,
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "BLE not initialized"),
            Self::AlreadyAdvertising => write!(f, "Already advertising"),
            Self::NotAdvertising => write!(f, "Not advertising"),
            Self::AdvertisingFailed => write!(f, "Advertising failed"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::InvalidParameter => write!(f, "Invalid parameter"),
            Self::Timeout => write!(f, "Timeout"),
            Self::InternalError => write!(f, "Internal error"),
            Self::MaxConnectionsReached => write!(f, "Max connections reached"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BleEvent {
    AdvertisingStarted,
    AdvertisingStopped,
    Connected {
        conn_handle: u16,
        peer_addr: [u8; 6],
    },
    Disconnected {
        conn_handle: u16,
        reason: DisconnectReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisconnectReason {
    #[default]
    Unknown,
    RemoteUserTerminated,
    LocalHostTerminated,
    ConnectionTimeout,
    AuthenticationFailure,
    UnacceptableConnectionParameters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BleState {
    #[default]
    Idle,
    Advertising,
    Connected,
}

#[derive(Debug, Clone)]
pub struct AdvertiseConfig {
    pub name: String<32>,
    pub interval_ms: u32,
    pub connectable: bool,
    pub scannable: bool,
    pub adv_data: Vec<u8, 31>,
    pub scan_rsp_data: Vec<u8, 31>,
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
    pub fn with_name(mut self, name: &str) -> Self {
        self.name.clear();
        let _ = self.name.push_str(name);
        self
    }

    pub fn with_interval_ms(mut self, interval: u32) -> Self {
        self.interval_ms = interval;
        self
    }

    pub fn with_connectable(mut self, connectable: bool) -> Self {
        self.connectable = connectable;
        self
    }

    pub fn with_timeout_secs(mut self, timeout: u16) -> Self {
        self.timeout_secs = timeout;
        self
    }

    pub fn with_adv_data(mut self, data: &[u8]) -> Self {
        self.adv_data.clear();
        let _ = self.adv_data.extend_from_slice(data);
        self
    }

    pub fn with_scan_rsp_data(mut self, data: &[u8]) -> Self {
        self.scan_rsp_data.clear();
        let _ = self.scan_rsp_data.extend_from_slice(data);
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    pub handle: u16,
    pub peer_addr: [u8; 6],
    pub interval: u16,
    pub latency: u16,
    pub timeout: u16,
    pub mtu: u16,
    pub bonded: bool,
}

#[cfg(feature = "ble")]
pub struct BleController<'a, C: Controller, P: PacketPool = DefaultPacketPool> {
    peripheral: Peripheral<'a, C, P>,
    runner: Option<Runner<'a, C, P>>,
    advertiser: Option<Advertiser<'a, C, P>>,
    connections: Vec<ConnectionInfo, BLE_MAX_CONNECTIONS>,
    events: Channel<CriticalSectionRawMutex, BleEvent, BLE_EVENT_QUEUE_SIZE>,
    state: BleState,
    local_addr: Address,
}

#[cfg(feature = "ble")]
fn map_host_error<E>(e: BleHostError<E>) -> BleError {
    match e {
        BleHostError::BleHost(Error::OutOfMemory) | BleHostError::BleHost(Error::InsufficientSpace) => {
            BleError::OutOfMemory
        }
        BleHostError::BleHost(Error::ConnectionLimitReached) => BleError::MaxConnectionsReached,
        BleHostError::BleHost(Error::Disconnected) => BleError::Disconnected,
        BleHostError::BleHost(Error::Timeout) => BleError::Timeout,
        BleHostError::BleHost(_) => BleError::AdvertisingFailed,
        BleHostError::Controller(_) => BleError::InternalError,
    }
}

#[cfg(feature = "ble")]
impl DisconnectReason {
    pub fn from_hci_status(code: u8) -> Self {
        match code {
            0x05 => Self::AuthenticationFailure,
            0x08 => Self::ConnectionTimeout,
            0x13 | 0x14 | 0x15 => Self::RemoteUserTerminated,
            0x16 => Self::LocalHostTerminated,
            0x0D | 0x0F | 0x10 | 0x1E | 0x20 => Self::UnacceptableConnectionParameters,
            _ => Self::Unknown,
        }
    }
}

#[cfg(feature = "ble")]
fn encode_adv_payload<'k>(
    config: &'k AdvertiseConfig,
    scratch: &'k mut [u8; 31],
) -> Result<(&'k [u8], &'k [u8]), BleError> {
    let adv: &[u8] = if config.adv_data.is_empty() {
        let name = config.name.as_bytes();
        let len = AdStructure::encode_slice(
            &[
                AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                AdStructure::CompleteLocalName(name),
            ],
            &mut scratch[..],
        )
        .map_err(|_| BleError::InvalidParameter)?;
        &scratch[..len]
    } else {
        &config.adv_data[..]
    };
    Ok((adv, &config.scan_rsp_data[..]))
}

#[cfg(feature = "ble")]
impl<'a, C: Controller, P: PacketPool> BleController<'a, C, P> {
    pub fn new(host: Host<'a, C, P>, local_address: Address) -> Self {
        let Host {
            peripheral,
            runner,
            ..
        } = host;
        Self {
            peripheral,
            runner: Some(runner),
            advertiser: None,
            connections: Vec::new(),
            events: Channel::new(),
            state: BleState::Idle,
            local_addr: local_address,
        }
    }

    pub fn take_runner(&mut self) -> Option<Runner<'a, C, P>> {
        self.runner.take()
    }

    pub fn peripheral_mut(&mut self) -> &mut Peripheral<'a, C, P> {
        &mut self.peripheral
    }

    pub fn state(&self) -> BleState {
        self.state
    }

    pub fn local_address(&self) -> Address {
        self.local_addr
    }

    pub async fn start_advertising(&mut self, config: &AdvertiseConfig) -> Result<(), BleError> {
        if self.advertiser.is_some() {
            return Err(BleError::AlreadyAdvertising);
        }

        let mut scratch = [0u8; 31];
        let (adv_data, scan_data) = encode_adv_payload(config, &mut scratch)?;

        let mut params = AdvertisementParameters::default();
        params.interval_min = core::time::Duration::from_millis(config.interval_ms as u64)
            .try_into()
            .map_err(|_| BleError::InvalidParameter)?;
        params.interval_max = params.interval_min;
        if config.timeout_secs > 0 {
            params.timeout = Some(
                core::time::Duration::from_secs(config.timeout_secs as u64)
                    .try_into()
                    .map_err(|_| BleError::InvalidParameter)?,
            );
        }

        let advertisement = if config.connectable {
            Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data,
            }
        } else if config.scannable {
            Advertisement::NonconnectableScannableUndirected {
                adv_data,
                scan_data,
            }
        } else {
            Advertisement::NonconnectableNonscannableUndirected { adv_data }
        };

        let advertiser = self
            .peripheral
            .advertise(&params, advertisement)
            .await
            .map_err(map_host_error)?;

        self.advertiser = Some(advertiser);
        self.state = BleState::Advertising;
        let _ = self.events.try_send(BleEvent::AdvertisingStarted);
        Ok(())
    }

    pub async fn update_advertising(&mut self, config: &AdvertiseConfig) -> Result<(), BleError> {
        let mut scratch = [0u8; 31];
        let (adv_data, scan_data) = encode_adv_payload(config, &mut scratch)?;
        self.peripheral
            .update_adv_data(Advertisement::ConnectableScannableUndirected {
                adv_data,
                scan_data,
            })
            .await
            .map_err(map_host_error)
    }

    pub async fn stop_advertising(&mut self) -> Result<(), BleError> {
        if let Some(advertiser) = self.advertiser.take() {
            drop(advertiser);
            if self.state == BleState::Advertising {
                self.state = BleState::Idle;
            }
            let _ = self.events.try_send(BleEvent::AdvertisingStopped);
        }
        Ok(())
    }

    pub async fn wait_for_connection(&mut self) -> Result<Connection<'a, P>, BleError> {
        let advertiser = self.advertiser.take().ok_or(BleError::NotAdvertising)?;

        let conn = advertiser
            .accept()
            .await
            .map_err(|e| match e {
                Error::Timeout => BleError::Timeout,
                Error::Disconnected => BleError::Disconnected,
                _ => BleError::ConnectionFailed,
            })?;

        let mut peer_addr = [0u8; 6];
        peer_addr.copy_from_slice(conn.peer_address().raw());
        let info = ConnectionInfo {
            handle: conn.handle().raw(),
            peer_addr,
            interval: BLE_CONN_INTERVAL_MIN,
            latency: BLE_SLAVE_LATENCY,
            timeout: BLE_SUPERVISION_TIMEOUT,
            mtu: conn.att_mtu(),
            bonded: false,
        };

        if self.connections.push(info.clone()).is_err() {
            conn.disconnect();
            self.state = if self.advertiser.is_some() {
                BleState::Advertising
            } else {
                BleState::Idle
            };
            return Err(BleError::MaxConnectionsReached);
        }

        self.state = BleState::Connected;
        let _ = self.events.try_send(BleEvent::Connected {
            conn_handle: info.handle,
            peer_addr: info.peer_addr,
        });
        Ok(conn)
    }

    pub fn note_disconnected(&mut self, conn_handle: u16, reason: DisconnectReason) {
        let before = self.connections.len();
        self.connections.retain(|c| c.handle != conn_handle);
        if self.connections.len() != before {
            let _ = self.events.try_send(BleEvent::Disconnected { conn_handle, reason });
        }
        if self.connections.is_empty() {
            self.state = if self.advertiser.is_some() {
                BleState::Advertising
            } else {
                BleState::Idle
            };
        }
    }

    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    pub fn connections(&self) -> &[ConnectionInfo] {
        &self.connections
    }

    pub fn connection_info(&self, handle: u16) -> Option<&ConnectionInfo> {
        self.connections.iter().find(|c| c.handle == handle)
    }

    pub async fn recv_event(&self) -> BleEvent {
        self.events.receive().await
    }

    pub fn try_recv_event(&self) -> Option<BleEvent> {
        self.events.try_receive().ok()
    }
}

#[cfg(feature = "ble")]
pub fn esp_ble_controller<'d>(
    radio: &'d esp_radio::Controller<'d>,
    bt: esp_hal::peripherals::BT<'d>,
) -> Result<ExternalController<BleConnector<'d>, 20>, BleError> {
    let connector = BleConnector::new(radio, bt, esp_radio::ble::Config::default())
        .map_err(|_| BleError::InvalidParameter)?;
    Ok(ExternalController::new(connector))
}

#[cfg(not(feature = "ble"))]
pub struct BleController<'a> {
    connector: BleConnector<'a>,
    state: BleState,
}

#[cfg(not(feature = "ble"))]
impl<'a> BleController<'a> {
    pub fn new(
        radio: &'a esp_radio::Controller<'a>,
        bt: esp_hal::peripherals::BT<'a>,
    ) -> Result<Self, BleError> {
        Self::with_config(radio, bt, esp_radio::ble::Config::default())
    }

    pub fn with_config(
        radio: &'a esp_radio::Controller<'a>,
        bt: esp_hal::peripherals::BT<'a>,
        config: esp_radio::ble::Config,
    ) -> Result<Self, BleError> {
        let connector =
            BleConnector::new(radio, bt, config).map_err(|_| BleError::InvalidParameter)?;
        Ok(Self {
            connector,
            state: BleState::Idle,
        })
    }

    pub fn state(&self) -> BleState {
        self.state
    }

    pub fn hci_mut(&mut self) -> &mut BleConnector<'a> {
        &mut self.connector
    }

    pub fn into_hci(self) -> BleConnector<'a> {
        self.connector
    }
}
