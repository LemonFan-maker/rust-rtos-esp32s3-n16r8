use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::{ConnectError, State as SocketState, TcpSocket};
use embassy_net::udp::{
    BindError as EmbBindError, PacketMetadata as UdpPacketMetadata, RecvError as EmbRecvError,
    SendError as EmbSendError, UdpSocket as EmbUdpSocket,
};
use embassy_net::{
    Config as EmbConfig, ConfigV4, DhcpConfig, IpAddress, Ipv4Cidr, Runner as EmbRunner,
    Stack as EmbStack, StackResources, StaticConfigV4,
};
use embassy_time::{with_timeout, Duration, Timer};
use esp_radio::wifi::WifiDevice;
use heapless::Vec as HeapVec;

use super::config::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    NotInitialized,
    ConnectionFailed,
    ConnectionRefused,
    ConnectionReset,
    Timeout,
    DnsResolutionFailed,
    InvalidAddress,
    SocketClosed,
    BufferFull,
    BufferEmpty,
    NetworkUnreachable,
    HostUnreachable,
    OutOfMemory,
    InternalError,
    NotConnected,
    AddressInUse,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "Network not initialized"),
            Self::ConnectionFailed => write!(f, "Connection failed"),
            Self::ConnectionRefused => write!(f, "Connection refused"),
            Self::ConnectionReset => write!(f, "Connection reset"),
            Self::Timeout => write!(f, "Timeout"),
            Self::DnsResolutionFailed => write!(f, "DNS resolution failed"),
            Self::InvalidAddress => write!(f, "Invalid address"),
            Self::SocketClosed => write!(f, "Socket closed"),
            Self::BufferFull => write!(f, "Buffer full"),
            Self::BufferEmpty => write!(f, "Buffer empty"),
            Self::NetworkUnreachable => write!(f, "Network unreachable"),
            Self::HostUnreachable => write!(f, "Host unreachable"),
            Self::OutOfMemory => write!(f, "Out of memory"),
            Self::InternalError => write!(f, "Internal error"),
            Self::NotConnected => write!(f, "Not connected"),
            Self::AddressInUse => write!(f, "Address in use"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);

    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);

    pub const BROADCAST: Self = Self([255, 255, 255, 255]);

    pub fn octets(&self) -> [u8; 4] {
        self.0
    }

    pub fn to_std(&self) -> Ipv4Addr {
        Ipv4Addr::new(self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

impl From<[u8; 4]> for Ipv4Address {
    fn from(octets: [u8; 4]) -> Self {
        Self(octets)
    }
}

impl From<Ipv4Addr> for Ipv4Address {
    fn from(addr: Ipv4Addr) -> Self {
        Self(addr.octets())
    }
}

fn netmask_to_prefix(mask: Ipv4Address) -> u8 {
    let bits = u32::from_be_bytes(mask.0);
    let ones = bits.count_ones();
    if ones == 32 || (ones > 0 && bits == u32::MAX >> (32 - ones)) {
        ones as u8
    } else {
        24
    }
}

fn endpoint_to_socketaddr(ep: embassy_net::IpEndpoint) -> Option<SocketAddrV4> {
    match ep.addr {
        IpAddress::Ipv4(v4) => Some(SocketAddrV4::new(v4, ep.port)),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackState {
    #[default]
    Uninitialized,
    NoIp,
    GettingIp,
    Ready,
}

#[derive(Debug, Clone)]
pub struct StackConfig {
    pub dhcp: bool,
    pub static_ip: Option<Ipv4Address>,
    pub netmask: Option<Ipv4Address>,
    pub gateway: Option<Ipv4Address>,
    pub dns: Option<Ipv4Address>,
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            dhcp: true,
            static_ip: None,
            netmask: None,
            gateway: None,
            dns: None,
        }
    }
}

impl StackConfig {
    pub fn with_static(ip: Ipv4Address, netmask: Ipv4Address, gateway: Ipv4Address) -> Self {
        Self {
            dhcp: false,
            static_ip: Some(ip),
            netmask: Some(netmask),
            gateway: Some(gateway),
            dns: Some(gateway),
        }
    }

    fn to_embassy(&self) -> EmbConfig {
        if self.dhcp {
            EmbConfig::dhcpv4(DhcpConfig::default())
        } else {
            let ip = self.static_ip.unwrap_or(Ipv4Address::UNSPECIFIED).to_std();
            let prefix = self
                .netmask
                .map(netmask_to_prefix)
                .unwrap_or(24);
            let gateway = self.gateway.map(|g| g.to_std());
            let mut dns: HeapVec<Ipv4Addr, 3> = HeapVec::new();
            if let Some(d) = self.dns.map(|x| x.to_std()).or(gateway) {
                let _ = dns.push(d);
            }
            EmbConfig::ipv4_static(StaticConfigV4 {
                address: Ipv4Cidr::new(ip, prefix),
                gateway,
                dns_servers: dns,
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct IpInfo {
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
    pub dns_servers: HeapVec<Ipv4Addr, 3>,
}

pub struct NetworkStack<'a> {
    stack: EmbStack<'a>,
    dhcp_requested: bool,
}

impl<'a> NetworkStack<'a> {
    pub fn new<const SOCK: usize>(
        device: WifiDevice<'a>,
        config: StackConfig,
        resources: &'a mut StackResources<SOCK>,
        random_seed: u64,
    ) -> (Self, EmbRunner<'a, WifiDevice<'a>>) {
        let dhcp_requested = config.dhcp;
        let (stack, runner) = embassy_net::new(
            device,
            config.to_embassy(),
            resources,
            random_seed,
        );
        (Self { stack, dhcp_requested }, runner)
    }

    pub fn stack(&self) -> EmbStack<'a> {
        self.stack
    }

    pub fn start_dhcp(&mut self) {
        self.dhcp_requested = true;
        self.stack.set_config_v4(ConfigV4::Dhcp(DhcpConfig::default()));
    }

    pub fn set_static_ip(
        &mut self,
        ip: Ipv4Address,
        netmask: Ipv4Address,
        gateway: Ipv4Address,
    ) {
        self.dhcp_requested = false;
        let mut dns: HeapVec<Ipv4Addr, 3> = HeapVec::new();
        let _ = dns.push(gateway.to_std());
        self.stack
            .set_config_v4(ConfigV4::Static(StaticConfigV4 {
                address: Ipv4Cidr::new(ip.to_std(), netmask_to_prefix(netmask)),
                gateway: Some(gateway.to_std()),
                dns_servers: dns,
            }));
    }

    pub fn state(&self) -> StackState {
        if self.stack.is_config_up() {
            StackState::Ready
        } else if self.dhcp_requested {
            StackState::GettingIp
        } else if self.stack.is_link_up() {
            StackState::NoIp
        } else {
            StackState::Uninitialized
        }
    }

    pub fn local_ip(&self) -> Option<Ipv4Address> {
        self.stack
            .config_v4()
            .map(|c| Ipv4Address::from(c.address.address()))
    }

    pub fn gateway(&self) -> Option<Ipv4Address> {
        self.stack
            .config_v4()
            .and_then(|c| c.gateway)
            .map(Ipv4Address::from)
    }

    pub fn dns_server(&self) -> Option<Ipv4Address> {
        self.stack
            .config_v4()
            .and_then(|c| c.dns_servers.first().copied())
            .map(Ipv4Address::from)
    }

    pub fn is_ready(&self) -> bool {
        self.stack.is_config_up()
    }

    pub fn is_link_up(&self) -> bool {
        self.stack.is_link_up()
    }

    pub fn mac_address(&self) -> Option<[u8; 6]> {
        match self.stack.hardware_address() {
            embassy_net::HardwareAddress::Ethernet(eth) => Some(eth.0),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    pub async fn wait_for_ip(&self) -> Result<IpInfo, NetworkError> {
        let timeout = Duration::from_secs(DHCP_TIMEOUT_SECS as u64);
        if with_timeout(timeout, self.stack.wait_config_up()).await.is_err() {
            return Err(NetworkError::Timeout);
        }

        let config = self.stack.config_v4().ok_or(NetworkError::Timeout)?;
        Ok(IpInfo {
            ip: config.address.address(),
            netmask: config.address.netmask(),
            gateway: config.gateway,
            dns_servers: config.dns_servers,
        })
    }

    pub async fn dns_resolve(&self, hostname: &str) -> Result<Ipv4Address, NetworkError> {
        if !self.stack.is_config_up() {
            return Err(NetworkError::NotInitialized);
        }

        let timeout = Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS as u64);
        let query = self.stack.dns_query(hostname, DnsQueryType::A);
        match with_timeout(timeout, query).await {
            Err(_) => Err(NetworkError::Timeout),
            Ok(Err(_)) => Err(NetworkError::DnsResolutionFailed),
            Ok(Ok(addrs)) => {
                for addr in addrs {
                    match addr {
                        IpAddress::Ipv4(v4) => return Ok(Ipv4Address::from(v4)),
                        #[allow(unreachable_patterns)]
                        _ => {}
                    }
                }
                Err(NetworkError::DnsResolutionFailed)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TcpState {
    #[default]
    Closed,
    Connecting,
    Connected,
    Closing,
}

fn map_socket_state(s: SocketState) -> TcpState {
    match s {
        SocketState::Closed | SocketState::TimeWait | SocketState::Listen => TcpState::Closed,
        SocketState::SynSent | SocketState::SynReceived => TcpState::Connecting,
        SocketState::Established | SocketState::CloseWait => TcpState::Connected,
        _ => TcpState::Closing,
    }
}

pub struct TcpClient<'a> {
    socket: TcpSocket<'a>,
}

impl<'a> TcpClient<'a> {
    pub fn new(stack: EmbStack<'a>, rx_buffer: &'a mut [u8], tx_buffer: &'a mut [u8]) -> Self {
        Self {
            socket: TcpSocket::new(stack, rx_buffer, tx_buffer),
        }
    }

    pub fn from_socket(socket: TcpSocket<'a>) -> Self {
        Self { socket }
    }

    pub async fn connect(&mut self, addr: SocketAddrV4) -> Result<(), NetworkError> {
        let timeout = Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS as u64);
        match with_timeout(timeout, self.socket.connect(addr)).await {
            Err(_) => Err(NetworkError::Timeout),
            Ok(Err(e)) => Err(match e {
                ConnectError::TimedOut => NetworkError::Timeout,
                ConnectError::ConnectionReset => NetworkError::ConnectionReset,
                ConnectError::NoRoute => NetworkError::HostUnreachable,
                ConnectError::InvalidState => NetworkError::InternalError,
            }),
            Ok(Ok(())) => Ok(()),
        }
    }

    pub async fn connect_to(&mut self, ip: Ipv4Address, port: u16) -> Result<(), NetworkError> {
        let addr = SocketAddrV4::new(ip.to_std(), port);
        self.connect(addr).await
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<usize, NetworkError> {
        let mut written = 0usize;
        while written < data.len() {
            match self.socket.write(&data[written..]).await {
                Ok(0) => {
                    return Err(NetworkError::SocketClosed);
                }
                Ok(n) => written += n,
                Err(_) => return Err(NetworkError::ConnectionReset),
            }
        }
        Ok(written)
    }

    pub async fn write_timeout(
        &mut self,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, NetworkError> {
        match with_timeout(timeout, self.write(data)).await {
            Err(_) => Err(NetworkError::Timeout),
            Ok(r) => r,
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError> {
        self.socket
            .read(buf)
            .await
            .map_err(|_| NetworkError::ConnectionReset)
    }

    pub async fn read_timeout(
        &mut self,
        buf: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, NetworkError> {
        match with_timeout(timeout, self.socket.read(buf)).await {
            Err(_) => Err(NetworkError::Timeout),
            Ok(Err(_)) => Err(NetworkError::ConnectionReset),
            Ok(Ok(n)) => Ok(n),
        }
    }

    pub async fn flush(&mut self) -> Result<(), NetworkError> {
        self.socket
            .flush()
            .await
            .map_err(|_| NetworkError::ConnectionReset)
    }

    pub async fn close(&mut self) -> Result<(), NetworkError> {
        self.socket.close();

        let wait = async {
            while !matches!(
                self.socket.state(),
                SocketState::Closed | SocketState::TimeWait
            ) {
                Timer::after(Duration::from_millis(50)).await;
            }
        };
        if with_timeout(Duration::from_secs(5), wait).await.is_err() {
            self.socket.abort();
        }
        Ok(())
    }

    pub fn abort(&mut self) {
        self.socket.abort();
    }

    pub fn state(&self) -> TcpState {
        map_socket_state(self.socket.state())
    }

    pub fn is_connected(&self) -> bool {
        self.state() == TcpState::Connected
    }

    pub fn remote_addr(&self) -> Option<SocketAddrV4> {
        self.socket
            .remote_endpoint()
            .and_then(endpoint_to_socketaddr)
    }

    pub fn local_port(&self) -> u16 {
        self.socket
            .local_endpoint()
            .map(|e| e.port)
            .unwrap_or(0)
    }
}

pub struct TcpServer {
    port: u16,
}

impl TcpServer {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn accept<'a>(
        &self,
        stack: EmbStack<'a>,
        rx_buffer: &'a mut [u8],
        tx_buffer: &'a mut [u8],
    ) -> Result<TcpClient<'a>, NetworkError> {
        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        match socket.accept(self.port).await {
            Ok(()) => Ok(TcpClient::from_socket(socket)),
            Err(e) => {
                drop(socket);
                Err(match e {
                    embassy_net::tcp::AcceptError::InvalidState => NetworkError::InternalError,
                    embassy_net::tcp::AcceptError::InvalidPort => NetworkError::AddressInUse,
                    embassy_net::tcp::AcceptError::ConnectionReset => {
                        NetworkError::ConnectionReset
                    }
                })
            }
        }
    }
}

pub struct UdpSocket<'a> {
    socket: EmbUdpSocket<'a>,
}

impl<'a> UdpSocket<'a> {
    pub fn new(
        stack: EmbStack<'a>,
        rx_meta: &'a mut [UdpPacketMetadata],
        rx_buffer: &'a mut [u8],
        tx_meta: &'a mut [UdpPacketMetadata],
        tx_buffer: &'a mut [u8],
    ) -> Self {
        Self {
            socket: EmbUdpSocket::new(stack, rx_meta, rx_buffer, tx_meta, tx_buffer),
        }
    }

    pub fn bind(&mut self, port: u16) -> Result<(), NetworkError> {
        self.socket.bind(port).map_err(|e| match e {
            EmbBindError::InvalidState => NetworkError::AddressInUse,
            EmbBindError::NoRoute => NetworkError::NetworkUnreachable,
        })
    }

    pub async fn send_to(&self, data: &[u8], addr: SocketAddrV4) -> Result<usize, NetworkError> {
        self.socket
            .send_to(data, addr)
            .await
            .map(|()| data.len())
            .map_err(|e| match e {
                EmbSendError::NoRoute => NetworkError::HostUnreachable,
                EmbSendError::SocketNotBound => NetworkError::NotConnected,
                EmbSendError::PacketTooLarge => NetworkError::BufferFull,
            })
    }

    pub async fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> Result<(usize, SocketAddrV4), NetworkError> {
        match self.socket.recv_from(buf).await {
            Ok((n, meta)) => {
                let addr = endpoint_to_socketaddr(meta.endpoint).ok_or(NetworkError::InvalidAddress)?;
                Ok((n, addr))
            }
            Err(EmbRecvError::Truncated) => Err(NetworkError::BufferFull),
        }
    }

    pub async fn recv_from_timeout(
        &self,
        buf: &mut [u8],
        timeout: Duration,
    ) -> Result<(usize, SocketAddrV4), NetworkError> {
        match with_timeout(timeout, self.recv_from(buf)).await {
            Err(_) => Err(NetworkError::Timeout),
            Ok(r) => r,
        }
    }

    pub fn local_port(&self) -> u16 {
        self.socket.endpoint().port
    }

    pub fn is_bound(&self) -> bool {
        self.socket.is_open()
    }

    pub fn close(&mut self) {
        self.socket.close();
    }
}
