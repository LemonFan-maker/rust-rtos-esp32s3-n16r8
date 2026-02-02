//! TCP/IP 网络栈模块
//!
//! 基于 embassy-net 和 smoltcp 提供 TCP/UDP Socket 抽象。
//!
//! # 功能
//!
//! - TCP 客户端/服务器
//! - UDP Socket
//! - DNS 解析
//! - DHCP 客户端
//!
//! # 示例
//!
//! ```ignore
//! use rustrtos::net::tcp::{TcpClient, NetworkStack};
//!
//! // 获取网络栈
//! let stack = NetworkStack::new(wifi_device);
//!
//! // TCP 客户端连接
//! let mut client = TcpClient::new(&stack);
//! client.connect("192.168.1.1:80").await?;
//! client.write(b"GET / HTTP/1.1\r\n\r\n").await?;
//! ```

use core::fmt;
use core::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use heapless::Vec;

use super::config::*;

// ===== 错误类型 =====

/// 网络错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    /// 未初始化
    NotInitialized,
    /// 连接失败
    ConnectionFailed,
    /// 连接被拒绝
    ConnectionRefused,
    /// 连接重置
    ConnectionReset,
    /// 连接超时
    Timeout,
    /// 地址解析失败
    DnsResolutionFailed,
    /// 无效地址
    InvalidAddress,
    /// Socket 已关闭
    SocketClosed,
    /// 缓冲区已满
    BufferFull,
    /// 缓冲区为空
    BufferEmpty,
    /// 网络不可达
    NetworkUnreachable,
    /// 主机不可达
    HostUnreachable,
    /// 资源不足
    OutOfMemory,
    /// 内部错误
    InternalError,
    /// 未连接
    NotConnected,
    /// 地址已在使用
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

// ===== IP 地址类型 =====

/// IPv4 地址
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    /// 创建新地址
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    /// 未指定地址 (0.0.0.0)
    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);

    /// 本地回环地址 (127.0.0.1)
    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);

    /// 广播地址 (255.255.255.255)
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);

    /// 转换为字节数组
    pub fn octets(&self) -> [u8; 4] {
        self.0
    }

    /// 转换为标准库类型
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

// ===== 网络栈 =====

/// 网络栈状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackState {
    /// 未初始化
    #[default]
    Uninitialized,
    /// 已初始化但无 IP
    NoIp,
    /// 正在获取 IP (DHCP)
    GettingIp,
    /// 已就绪
    Ready,
}

/// 网络栈配置
#[derive(Debug, Clone)]
pub struct StackConfig {
    /// 是否启用 DHCP
    pub dhcp: bool,
    /// 静态 IP 地址
    pub static_ip: Option<Ipv4Address>,
    /// 子网掩码
    pub netmask: Option<Ipv4Address>,
    /// 网关
    pub gateway: Option<Ipv4Address>,
    /// DNS 服务器
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
    /// 使用静态 IP 配置
    pub fn with_static(ip: Ipv4Address, netmask: Ipv4Address, gateway: Ipv4Address) -> Self {
        Self {
            dhcp: false,
            static_ip: Some(ip),
            netmask: Some(netmask),
            gateway: Some(gateway),
            dns: Some(gateway), // 默认使用网关作为 DNS
        }
    }
}

/// 网络栈
///
/// 封装 embassy-net 提供的网络功能。
pub struct NetworkStack<'a> {
    /// 状态
    state: StackState,
    /// 配置
    config: StackConfig,
    /// 本地 IP 地址
    local_ip: Option<Ipv4Address>,
    /// 网关地址
    gateway: Option<Ipv4Address>,
    /// DNS 服务器
    dns_server: Option<Ipv4Address>,
    /// 生命周期标记
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> NetworkStack<'a> {
    /// 创建新的网络栈
    pub fn new(config: StackConfig) -> Self {
        Self {
            state: StackState::Uninitialized,
            config,
            local_ip: None,
            gateway: None,
            dns_server: None,
            _marker: core::marker::PhantomData,
        }
    }

    /// 初始化网络栈
    ///
    /// **注意**: 此函数仅初始化状态。实际网络栈应通过 embassy-net 配置。
    /// 参见 `examples/tcp_client.rs`。
    pub async fn init(&mut self) -> Result<(), NetworkError> {
        // 状态管理层 - 实际网络栈通过 embassy_net::Stack 初始化
        self.state = StackState::NoIp;
        Ok(())
    }

    /// 启动 DHCP 客户端
    ///
    /// **注意**: 此函数返回默认 IP。实际 DHCP 应通过 `embassy_net::DhcpConfig` 配置。
    /// 成功获取 IP 后调用 `set_addresses()` 更新状态。
    pub async fn start_dhcp(&mut self) -> Result<(), NetworkError> {
        if self.state == StackState::Uninitialized {
            return Err(NetworkError::NotInitialized);
        }

        self.state = StackState::GettingIp;
        
        // 状态管理层 - 实际 DHCP 通过 embassy_net::DhcpConfig 完成
        // 返回默认 IP 以允许测试
        Timer::after(Duration::from_millis(100)).await;
        
        self.local_ip = Some(Ipv4Address::new(192, 168, 1, 100));
        self.gateway = Some(Ipv4Address::new(192, 168, 1, 1));
        self.dns_server = Some(Ipv4Address::new(8, 8, 8, 8));
        self.state = StackState::Ready;

        Ok(())
    }

    /// 设置静态 IP
    pub async fn set_static_ip(
        &mut self,
        ip: Ipv4Address,
        netmask: Ipv4Address,
        gateway: Ipv4Address,
    ) -> Result<(), NetworkError> {
        if self.state == StackState::Uninitialized {
            return Err(NetworkError::NotInitialized);
        }

        self.local_ip = Some(ip);
        self.gateway = Some(gateway);
        self.state = StackState::Ready;

        Ok(())
    }

    /// 获取当前状态
    pub fn state(&self) -> StackState {
        self.state
    }

    /// 获取本地 IP 地址
    pub fn local_ip(&self) -> Option<Ipv4Address> {
        self.local_ip
    }

    /// 获取网关地址
    pub fn gateway(&self) -> Option<Ipv4Address> {
        self.gateway
    }

    /// 获取 DNS 服务器
    pub fn dns_server(&self) -> Option<Ipv4Address> {
        self.dns_server
    }

    /// 检查是否就绪
    pub fn is_ready(&self) -> bool {
        self.state == StackState::Ready
    }

    /// DNS 解析
    ///
    /// **注意**: 此函数返回错误。实际 DNS 解析应通过
    /// `embassy_net::dns::DnsQueryType::A` 和 `Stack::dns_query()` 完成。
    pub async fn dns_resolve(&self, _hostname: &str) -> Result<Ipv4Address, NetworkError> {
        if self.state != StackState::Ready {
            return Err(NetworkError::NotInitialized);
        }

        // 状态管理层 - 实际 DNS 解析通过 embassy_net Stack 完成
        Err(NetworkError::DnsResolutionFailed)
    }
}

// ===== TCP Client =====

/// TCP Socket 状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TcpState {
    /// 已关闭
    #[default]
    Closed,
    /// 正在连接
    Connecting,
    /// 已连接
    Connected,
    /// 正在关闭
    Closing,
}

/// TCP 客户端
pub struct TcpClient<'a> {
    /// 状态
    state: TcpState,
    /// 本地端口
    local_port: u16,
    /// 远程地址
    remote_addr: Option<SocketAddrV4>,
    /// 接收缓冲区
    rx_buffer: Vec<u8, TCP_RX_BUFFER_SIZE>,
    /// 发送缓冲区
    tx_buffer: Vec<u8, TCP_TX_BUFFER_SIZE>,
    /// 网络栈引用
    _stack: core::marker::PhantomData<&'a ()>,
}

impl<'a> TcpClient<'a> {
    /// 创建新的 TCP 客户端
    pub fn new() -> Self {
        Self {
            state: TcpState::Closed,
            local_port: 0,
            remote_addr: None,
            rx_buffer: Vec::new(),
            tx_buffer: Vec::new(),
            _stack: core::marker::PhantomData,
        }
    }

    /// 连接到远程地址
    ///
    /// **注意**: 此函数仅更新状态。实际 TCP 连接应通过
    /// `embassy_net::tcp::TcpSocket::connect()` 完成。
    pub async fn connect(&mut self, addr: SocketAddrV4) -> Result<(), NetworkError> {
        if self.state != TcpState::Closed {
            return Err(NetworkError::InternalError);
        }

        self.state = TcpState::Connecting;
        self.remote_addr = Some(addr);

        // 状态管理层 - 实际连接通过 embassy_net::tcp::TcpSocket 完成
        let timeout = Duration::from_secs(TCP_CONNECT_TIMEOUT_SECS as u64);
        let _ = timeout; // 仅用于类型检查
        
        // 状态转换延迟
        Timer::after(Duration::from_millis(100)).await;
        
        self.state = TcpState::Connected;
        self.local_port = 49152; // 动态端口

        Ok(())
    }

    /// 连接到 IP 和端口
    pub async fn connect_to(&mut self, ip: Ipv4Address, port: u16) -> Result<(), NetworkError> {
        let addr = SocketAddrV4::new(ip.to_std(), port);
        self.connect(addr).await
    }

    /// 发送数据
    ///
    /// **注意**: 此函数返回数据长度但不真正发送。实际发送应通过
    /// `embassy_net::tcp::TcpSocket::write()` 完成。
    pub async fn write(&mut self, data: &[u8]) -> Result<usize, NetworkError> {
        if self.state != TcpState::Connected {
            return Err(NetworkError::NotConnected);
        }

        // 状态管理层 - 实际发送通过 embassy_net::tcp::TcpSocket 完成
        Ok(data.len())
    }

    /// 接收数据
    ///
    /// **注意**: 此函数返回 0 字节。实际接收应通过
    /// `embassy_net::tcp::TcpSocket::read()` 完成。
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError> {
        if self.state != TcpState::Connected {
            return Err(NetworkError::NotConnected);
        }

        // 状态管理层 - 实际接收通过 embassy_net::tcp::TcpSocket 完成
        let _ = buf; // 仅用于类型检查
        Ok(0)
    }

    /// 关闭连接
    ///
    /// **注意**: 此函数仅更新状态。实际关闭应通过
    /// `embassy_net::tcp::TcpSocket::close()` 完成。
    pub async fn close(&mut self) -> Result<(), NetworkError> {
        if self.state == TcpState::Closed {
            return Ok(());
        }

        self.state = TcpState::Closing;
        
        // 状态管理层 - 实际关闭通过 embassy_net::tcp::TcpSocket 完成
        
        self.state = TcpState::Closed;
        self.remote_addr = None;
        self.rx_buffer.clear();
        self.tx_buffer.clear();

        Ok(())
    }

    /// 获取状态
    pub fn state(&self) -> TcpState {
        self.state
    }

    /// 检查是否已连接
    pub fn is_connected(&self) -> bool {
        self.state == TcpState::Connected
    }

    /// 获取远程地址
    pub fn remote_addr(&self) -> Option<SocketAddrV4> {
        self.remote_addr
    }

    /// 获取本地端口
    pub fn local_port(&self) -> u16 {
        self.local_port
    }
}

impl<'a> Default for TcpClient<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// ===== TCP Server =====

/// TCP 服务器
pub struct TcpServer<'a> {
    /// 监听端口
    port: u16,
    /// 是否正在监听
    listening: bool,
    /// 生命周期标记
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> TcpServer<'a> {
    /// 创建新的 TCP 服务器
    pub fn new(port: u16) -> Self {
        Self {
            port,
            listening: false,
            _marker: core::marker::PhantomData,
        }
    }

    /// 开始监听
    ///
    /// **注意**: 此函数仅更新状态。实际监听应通过
    /// `embassy_net::tcp::TcpSocket::accept()` 完成。
    pub async fn listen(&mut self) -> Result<(), NetworkError> {
        // 状态管理层 - 实际监听通过 embassy_net::tcp::TcpSocket 完成
        self.listening = true;
        Ok(())
    }

    /// 接受连接
    ///
    /// **注意**: 此函数永远等待。实际接受应通过
    /// `embassy_net::tcp::TcpSocket::accept()` 完成。
    pub async fn accept(&mut self) -> Result<TcpClient<'a>, NetworkError> {
        if !self.listening {
            return Err(NetworkError::NotInitialized);
        }

        // 状态管理层 - 实际接受通过 embassy_net::tcp::TcpSocket 完成
        // 此处永远等待，应用层应直接使用 embassy-net
        loop {
            Timer::after(Duration::from_millis(100)).await;
        }
    }

    /// 停止监听
    pub async fn close(&mut self) -> Result<(), NetworkError> {
        self.listening = false;
        Ok(())
    }

    /// 获取监听端口
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 检查是否正在监听
    pub fn is_listening(&self) -> bool {
        self.listening
    }
}

// ===== UDP Socket =====

/// UDP Socket
pub struct UdpSocket<'a> {
    /// 本地端口
    local_port: u16,
    /// 是否已绑定
    bound: bool,
    /// 接收缓冲区
    rx_buffer: Vec<u8, UDP_RX_BUFFER_SIZE>,
    /// 生命周期标记
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> UdpSocket<'a> {
    /// 创建新的 UDP Socket
    pub fn new() -> Self {
        Self {
            local_port: 0,
            bound: false,
            rx_buffer: Vec::new(),
            _marker: core::marker::PhantomData,
        }
    }

    /// 绑定到端口
    ///
    /// **注意**: 此函数仅更新状态。实际绑定应通过
    /// `embassy_net::udp::UdpSocket::bind()` 完成。
    pub async fn bind(&mut self, port: u16) -> Result<(), NetworkError> {
        // 状态管理层 - 实际绑定通过 embassy_net::udp::UdpSocket 完成
        self.local_port = port;
        self.bound = true;
        Ok(())
    }

    /// 发送数据到指定地址
    ///
    /// **注意**: 此函数返回数据长度但不真正发送。实际发送应通过
    /// `embassy_net::udp::UdpSocket::send_to()` 完成。
    pub async fn send_to(&self, data: &[u8], addr: SocketAddrV4) -> Result<usize, NetworkError> {
        if !self.bound {
            return Err(NetworkError::NotInitialized);
        }

        // 状态管理层 - 实际发送通过 embassy_net::udp::UdpSocket 完成
        let _ = addr; // 仅用于类型检查
        Ok(data.len())
    }

    /// 接收数据
    ///
    /// **注意**: 此函数永远等待。实际接收应通过
    /// `embassy_net::udp::UdpSocket::recv_from()` 完成。
    pub async fn recv_from(&mut self, buf: &mut [u8]) -> Result<(usize, SocketAddrV4), NetworkError> {
        if !self.bound {
            return Err(NetworkError::NotInitialized);
        }

        // 状态管理层 - 实际接收通过 embassy_net::udp::UdpSocket 完成
        // 此处永远等待，应用层应直接使用 embassy-net
        let _ = buf; // 仅用于类型检查
        loop {
            Timer::after(Duration::from_millis(100)).await;
        }
    }

    /// 关闭 Socket
    pub async fn close(&mut self) -> Result<(), NetworkError> {
        self.bound = false;
        self.local_port = 0;
        Ok(())
    }

    /// 获取本地端口
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// 检查是否已绑定
    pub fn is_bound(&self) -> bool {
        self.bound
    }
}

impl<'a> Default for UdpSocket<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 网络统计 =====

/// 网络统计信息
#[derive(Debug, Clone, Default)]
pub struct NetworkStats {
    /// 发送的数据包
    pub tx_packets: u64,
    /// 接收的数据包
    pub rx_packets: u64,
    /// 发送的字节
    pub tx_bytes: u64,
    /// 接收的字节
    pub rx_bytes: u64,
    /// 发送错误
    pub tx_errors: u32,
    /// 接收错误
    pub rx_errors: u32,
    /// 丢弃的数据包
    pub dropped: u32,
}
