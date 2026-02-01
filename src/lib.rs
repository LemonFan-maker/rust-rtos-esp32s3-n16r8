//! RustRTOS - ESP32-S3 高性能实时操作系统库
//!
//! 本库提供以下核心功能:
//! - 多优先级任务调度 (基于 Embassy + esp-rtos)
//! - 双核 SMP 支持
//! - PSRAM 内存管理
//! - 内存池分配器
//! - DMA 缓冲区管理
//! - LittleFS 文件系统
//! - 零拷贝同步原语
//! - 高性能环形缓冲区
//! - 条件编译日志系统

#![no_std]
#![feature(asm_experimental_arch)]

pub mod tasks;
pub mod sync;
pub mod util;
pub mod mem;
pub mod fs;

// ===== 重导出常用类型 =====
pub use sync::primitives::{
    CriticalMutex,
    CriticalSignal,
    CriticalChannel,
};
pub use sync::ringbuffer::RingBuffer;

// 内存管理重导出
pub use mem::{
    psram::{CacheMode, PsramBox, PsramConfig, PsramInfo, PsramError, PsramStats},
    pool::{MemoryPool, PoolBox},
    dma::{DmaBuffer, DmaStrategy},
};

// 多核支持重导出
pub use tasks::multicore::{
    CoreId, CoreAssignment, Core1,
    IpcChannel, IpcSignal, IpcSemaphore,
};

// 文件系统重导出
pub use fs::{
    FileSystem, File, OpenOptions, FileType, Metadata,
    PartitionTable, Partition, PartitionType,
    FlashStorage, StorageError,
};


// ===== 版本信息 =====
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// 系统配置常量
pub mod config {
    /// CPU 频率 (Hz)
    pub const CPU_FREQ_HZ: u32 = 240_000_000;
    
    /// 系统 Tick 频率 (Hz) - Embassy 配置为 1MHz
    pub const TICK_FREQ_HZ: u32 = 1_000_000;
    
    /// 高优先级中断等级 (ESP32-S3 Xtensa 最高为 Priority3)
    pub const HIGH_PRIORITY: u8 = 3;
    
    /// 中优先级中断等级
    pub const MID_PRIORITY: u8 = 2;
    
    /// 低优先级中断等级
    pub const LOW_PRIORITY: u8 = 1;
    
    /// 核间通信中断等级
    pub const IPC_PRIORITY: u8 = 2;
    
    /// 默认任务栈大小 (字节)
    pub const DEFAULT_STACK_SIZE: usize = 4096;
    
    /// 最小任务栈大小 (字节)
    pub const MIN_STACK_SIZE: usize = 512;
    
    /// 环形缓冲区默认大小
    pub const DEFAULT_RINGBUF_SIZE: usize = 256;
    
    /// PSRAM 基地址
    pub const PSRAM_BASE: u32 = 0x3C000000;
    
    /// PSRAM 大小 (8MB for N16R8)
    pub const PSRAM_SIZE: usize = 8 * 1024 * 1024;
    
    /// DMA 缓冲区对齐 (cache line)
    pub const DMA_ALIGNMENT: usize = 32;
    
    /// 默认 Flash 块大小
    pub const FLASH_BLOCK_SIZE: u32 = 4096;
}
