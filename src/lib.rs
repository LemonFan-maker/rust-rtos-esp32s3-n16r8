#![no_std]
#![feature(asm_experimental_arch)]

pub mod tasks;
pub mod apps;
pub mod sync;
pub mod util;
pub mod mem;
pub mod fs;
pub mod ota;
pub mod perf;
pub mod watchdog;
pub mod runtime;

#[cfg(any(feature = "wifi", feature = "ble", feature = "ble-esp"))]
pub mod net;

#[cfg(all(feature = "ble", feature = "ble-esp"))]
compile_error!("feature \"ble\" 与 \"ble-esp\" 互斥, 只能启用其一");

pub use sync::primitives::{
    CriticalMutex,
    CriticalSignal,
    CriticalChannel,
};
pub use sync::ringbuffer::RingBuffer;

pub use mem::{
    psram::{PsramBox, PsramConfig, PsramInfo, PsramError, PsramStats},
    pool::{MemoryPool, PoolBox},
    dma::DmaBuffer,
    gdma::{copy_blocking, GdmaCopy},
};

pub use tasks::multicore::{
    CoreId, CoreAssignment, Core1,
    IpcChannel, IpcSignal, IpcSemaphore,
};

pub use fs::{
    FileSystem, File, OpenOptions, FileType, Metadata, MountPolicy, VolumeState,
    PartitionTable, Partition, PartitionType,
    FlashStorage, StorageError,
};
pub use ota::{mark_current_valid, validate_image_header, OtaError, OtaSession, OtaUpdate};

#[cfg(feature = "wifi")]
pub use net::wifi::{WifiController, WifiMode, WifiEvent, WifiError, WifiState, ScanResult};

#[cfg(any(feature = "ble", feature = "ble-esp"))]
pub use net::ble::{BleController, BleEvent, BleError, BleState, AdvertiseConfig};

#[cfg(feature = "network")]
pub use net::tcp::{TcpClient, TcpServer, UdpSocket, NetworkStack, NetworkError};

#[cfg(any(feature = "wifi", feature = "ble", feature = "ble-esp"))]
pub use net::config::NetworkConfig;
pub use runtime::Runtime;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");

pub mod config {
    pub const CPU_FREQ_HZ: u32 = 240_000_000;

    pub const TICK_FREQ_HZ: u32 = 1_000_000;

    pub const HIGH_PRIORITY: u8 = 3;

    pub const MID_PRIORITY: u8 = 2;

    pub const LOW_PRIORITY: u8 = 1;

    pub const IPC_PRIORITY: u8 = 2;

    pub const DEFAULT_STACK_SIZE: usize = 4096;

    pub const MIN_STACK_SIZE: usize = 512;

    pub const DEFAULT_RINGBUF_SIZE: usize = 256;

    pub const PSRAM_BASE: u32 = 0x3C000000;

    pub const PSRAM_SIZE: usize = 8 * 1024 * 1024;

    pub const DMA_ALIGNMENT: usize = 32;

    pub const FLASH_BLOCK_SIZE: u32 = 4096;
}
