use core::fmt;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Duration};
use heapless::{String, Vec};

use esp_radio::wifi::{
    AccessPointConfig as EspApConfig, AuthMethod, ClientConfig as EspClientConfig, ModeConfig,
    WifiDevice, WifiError as EspWifiError, WifiEvent as EspWifiEvent,
};
use esp_radio::Controller as RadioController;

use super::config::*;

pub const MAX_SCAN_RESULTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiError {
    NotInitialized,
    ConnectionFailed,
    AuthenticationFailed,
    NetworkNotFound,
    Timeout,
    Disconnected,
    InternalError,
    ConfigError,
    ScanFailed,
    OutOfMemory,
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

fn map_esp_error(e: EspWifiError) -> WifiError {
    match e {
        EspWifiError::NotInitialized => WifiError::NotInitialized,
        EspWifiError::Disconnected => WifiError::Disconnected,
        EspWifiError::InvalidArguments => WifiError::ConfigError,
        EspWifiError::Unsupported => WifiError::Unsupported,
        EspWifiError::UnknownWifiMode => WifiError::ConfigError,
        EspWifiError::InternalError(_) => WifiError::InternalError,
        #[allow(unreachable_patterns)]
        _ => WifiError::InternalError,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiMode {
    #[default]
    None,
    Sta,
    Ap,
    ApSta,
}

#[derive(Debug, Clone)]
pub enum WifiEvent {
    StaConnected,
    StaDisconnected {
        reason: DisconnectReason,
    },
    GotIp {
        ip: [u8; 4],
        gateway: [u8; 4],
        netmask: [u8; 4],
    },
    ScanDone {
        count: usize,
    },
    ApStaConnected {
        mac: [u8; 6],
    },
    ApStaDisconnected {
        mac: [u8; 6],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Unspecified,
    AuthExpired,
    AuthLeave,
    AssocExpired,
    AssocTooMany,
    NotAuthenticated,
    NotAssociated,
    AssocLeave,
    AssocNotAuth,
    BadChannel,
    BeaconTimeout,
    NoApFound,
    WrongPassword,
    ConnectionFail,
    ApHandshakeFail,
}

impl Default for DisconnectReason {
    fn default() -> Self {
        Self::Unspecified
    }
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub ssid: String<32>,
    pub bssid: [u8; 6],
    pub rssi: i8,
    pub channel: u8,
    pub auth_mode: AuthMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    Open,
    Wep,
    WpaPsk,
    Wpa2Psk,
    WpaWpa2Psk,
    Wpa3Psk,
    Wpa2Wpa3Psk,
    Enterprise,
    WapiPsk,
    Other,
}

fn map_auth(auth: Option<AuthMethod>) -> AuthMode {
    match auth {
        None => AuthMode::Open,
        Some(AuthMethod::None) => AuthMode::Open,
        Some(AuthMethod::Wep) => AuthMode::Wep,
        Some(AuthMethod::Wpa) => AuthMode::WpaPsk,
        Some(AuthMethod::Wpa2Personal) => AuthMode::Wpa2Psk,
        Some(AuthMethod::WpaWpa2Personal) => AuthMode::WpaWpa2Psk,
        Some(AuthMethod::Wpa2Enterprise) => AuthMode::Enterprise,
        Some(AuthMethod::Wpa3Personal) => AuthMode::Wpa3Psk,
        Some(AuthMethod::Wpa2Wpa3Personal) => AuthMode::Wpa2Wpa3Psk,
        Some(AuthMethod::WapiPersonal) => AuthMode::WapiPsk,
        #[allow(unreachable_patterns)]
        Some(_) => AuthMode::Other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiState {
    #[default]
    Uninitialized,
    Idle,
    Scanning,
    Connecting,
    Connected,
    GettingIp,
    Ready,
    Disconnected,
}

pub struct WifiController<'a> {
    inner: esp_radio::wifi::WifiController<'a>,
    sta_device: Option<WifiDevice<'a>>,
    ap_device: Option<WifiDevice<'a>>,
    mode: WifiMode,
    state: WifiState,
    started: bool,
    ssid: String<32>,
    password: String<64>,
    ap_config: ApConfig,
    ip_address: Option<[u8; 4]>,
    gateway: Option<[u8; 4]>,
    event_channel: &'a Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    scan_results: Vec<ScanResult, MAX_SCAN_RESULTS>,
}

impl<'a> WifiController<'a> {
    pub fn new(
        radio: &'a RadioController<'a>,
        wifi_peripheral: esp_hal::peripherals::WIFI<'a>,
        event_channel: &'a Channel<CriticalSectionRawMutex, WifiEvent, WIFI_EVENT_QUEUE_SIZE>,
    ) -> Result<Self, WifiError> {
        let (inner, itf) =
            esp_radio::wifi::new(radio, wifi_peripheral, Default::default()).map_err(map_esp_error)?;
        Ok(Self {
            inner,
            sta_device: Some(itf.sta),
            ap_device: Some(itf.ap),
            mode: WifiMode::None,
            state: WifiState::Idle,
            started: false,
            ssid: String::new(),
            password: String::new(),
            ap_config: ApConfig::default(),
            ip_address: None,
            gateway: None,
            event_channel,
            scan_results: Vec::new(),
        })
    }

    pub fn take_sta(&mut self) -> Option<WifiDevice<'a>> {
        self.sta_device.take()
    }

    pub fn take_ap(&mut self) -> Option<WifiDevice<'a>> {
        self.ap_device.take()
    }

    fn build_client_config(&self) -> EspClientConfig {
        EspClientConfig::default()
            .with_ssid(self.ssid.as_str().into())
            .with_password(self.password.as_str().into())
    }

    fn build_ap_config(&self) -> EspApConfig {
        let auth = if self.ap_config.password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::Wpa2Personal
        };
        EspApConfig::default()
            .with_ssid(self.ap_config.ssid.as_str().into())
            .with_password(self.ap_config.password.as_str().into())
            .with_channel(self.ap_config.channel)
            .with_ssid_hidden(self.ap_config.hidden)
            .with_max_connections(self.ap_config.max_clients as u16)
            .with_auth_method(auth)
    }

    async fn ensure_started(&mut self) -> Result<(), WifiError> {
        if !self.started {
            self.inner.start_async().await.map_err(map_esp_error)?;
            self.started = true;
        }
        Ok(())
    }

    pub async fn set_mode(&mut self, mode: WifiMode) -> Result<(), WifiError> {
        let conf = match mode {
            WifiMode::None => ModeConfig::None,
            WifiMode::Sta => ModeConfig::Client(self.build_client_config()),
            WifiMode::Ap => ModeConfig::AccessPoint(self.build_ap_config()),
            WifiMode::ApSta => {
                ModeConfig::ApSta(self.build_client_config(), self.build_ap_config())
            }
        };

        self.inner.set_config(&conf).map_err(map_esp_error)?;
        self.mode = mode;

        if mode != WifiMode::None {
            self.ensure_started().await?;
        }
        Ok(())
    }

    pub fn mode(&self) -> WifiMode {
        self.mode
    }

    pub fn state(&self) -> WifiState {
        self.state
    }

    pub fn set_ap_config(&mut self, config: ApConfig) {
        self.ap_config = config;
    }

    pub fn ap_config(&self) -> &ApConfig {
        &self.ap_config
    }

    pub async fn scan(&mut self) -> Result<&[ScanResult], WifiError> {
        if self.mode == WifiMode::None {
            self.inner
                .set_config(&ModeConfig::Client(EspClientConfig::default()))
                .map_err(map_esp_error)?;
            self.mode = WifiMode::Sta;
        }
        self.ensure_started().await?;

        let was_connected = self.is_connected();
        self.state = WifiState::Scanning;
        self.scan_results.clear();

        let scan_fut = self.inner.scan_with_config_async(Default::default());
        let timeout = Duration::from_millis(WIFI_SCAN_TIMEOUT_MS as u64);
        let result = match with_timeout(timeout, scan_fut).await {
            Err(_) => {
                self.state = if was_connected { WifiState::Connected } else { WifiState::Idle };
                return Err(WifiError::Timeout);
            }
            Ok(Err(e)) => {
                self.state = if was_connected { WifiState::Connected } else { WifiState::Idle };
                return Err(map_esp_error(e));
            }
            Ok(Ok(aps)) => aps,
        };

        for ap in result {
            if self.scan_results.is_full() {
                break;
            }
            let mut ssid: String<32> = String::new();
            for ch in ap.ssid.chars().take(32) {
                let _ = ssid.push(ch);
            }
            let _ = self.scan_results.push(ScanResult {
                ssid,
                bssid: ap.bssid,
                rssi: ap.signal_strength,
                channel: ap.channel,
                auth_mode: map_auth(ap.auth_method),
            });
        }

        self.state = if was_connected { WifiState::Connected } else { WifiState::Idle };

        let _ = self.event_channel.try_send(WifiEvent::ScanDone {
            count: self.scan_results.len(),
        });

        Ok(&self.scan_results)
    }

    pub async fn connect(&mut self, ssid: &str, password: &str) -> Result<(), WifiError> {
        if ssid.len() > 32 || password.len() > 64 {
            return Err(WifiError::ConfigError);
        }

        self.ssid.clear();
        let _ = self.ssid.push_str(ssid);
        self.password.clear();
        let _ = self.password.push_str(password);

        self.inner
            .set_config(&ModeConfig::Client(self.build_client_config()))
            .map_err(map_esp_error)?;
        self.mode = WifiMode::Sta;
        self.ensure_started().await?;

        self.state = WifiState::Connecting;

        let timeout = Duration::from_millis(WIFI_CONNECT_TIMEOUT_MS as u64);
        match with_timeout(timeout, self.inner.connect_async()).await {
            Err(_) => {
                self.state = WifiState::Disconnected;
                Err(WifiError::Timeout)
            }
            Ok(Err(e)) => {
                self.state = WifiState::Disconnected;
                let mapped = map_esp_error(e);
                let _ = self.event_channel.try_send(WifiEvent::StaDisconnected {
                    reason: DisconnectReason::ConnectionFail,
                });
                Err(mapped)
            }
            Ok(Ok(())) => {
                self.state = WifiState::Connected;
                let _ = self.event_channel.try_send(WifiEvent::StaConnected);
                Ok(())
            }
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), WifiError> {
        if !self.started {
            return Err(WifiError::NotInitialized);
        }

        self.inner.disconnect_async().await.map_err(map_esp_error)?;

        self.state = WifiState::Disconnected;
        self.ip_address = None;
        self.gateway = None;

        let _ = self.event_channel.try_send(WifiEvent::StaDisconnected {
            reason: DisconnectReason::AssocLeave,
        });

        Ok(())
    }

    pub fn ip_address(&self) -> Option<[u8; 4]> {
        self.ip_address
    }

    pub fn gateway(&self) -> Option<[u8; 4]> {
        self.gateway
    }

    pub fn set_ip_address(&mut self, ip: [u8; 4], gateway: [u8; 4]) {
        self.ip_address = Some(ip);
        self.gateway = Some(gateway);
        if self.state == WifiState::Connected || self.state == WifiState::GettingIp {
            self.state = WifiState::Ready;
        }

        let _ = self.event_channel.try_send(WifiEvent::GotIp {
            ip,
            gateway,
            netmask: [255, 255, 255, 0],
        });
    }

    pub fn is_connected(&self) -> bool {
        self.inner.is_connected().unwrap_or(false)
    }

    pub fn rssi(&self) -> Result<i8, WifiError> {
        self.inner
            .rssi()
            .map(|r| r.clamp(i8::MIN as i32, i8::MAX as i32) as i8)
            .map_err(map_esp_error)
    }

    pub fn mac_address(&self) -> [u8; 6] {
        esp_radio::wifi::sta_mac()
    }

    pub async fn wait_for_event(&mut self, event: EspWifiEvent) {
        self.inner.wait_for_event(event).await
    }

    pub fn scan_results(&self) -> &[ScanResult] {
        &self.scan_results
    }

    pub async fn recv_event(&self) -> WifiEvent {
        self.event_channel.receive().await
    }

    pub fn try_recv_event(&self) -> Option<WifiEvent> {
        self.event_channel.try_receive().ok()
    }
}

#[derive(Debug, Clone)]
pub struct ApConfig {
    pub ssid: String<32>,
    pub password: String<64>,
    pub channel: u8,
    pub max_clients: u8,
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
