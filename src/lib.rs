//! RustRTOS - ESP32-S3 高性能实时操作系统库
//!
//! 本库提供以下核心功能:
//! - 多优先级任务调度 (基于 Embassy)
//! - 零拷贝同步原语
//! - 高性能环形缓冲区
//! - 条件编译日志系统

#![no_std]

pub mod tasks;
pub mod sync;
pub mod util;

// ===== 重导出常用类型 =====
pub use sync::primitives::{
    CriticalMutex,
    CriticalSignal,
    CriticalChannel,
};
pub use sync::ringbuffer::RingBuffer;

// ===== 版本信息 =====
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// 系统配置常量
pub mod config {
    /// CPU 频率 (Hz)
    pub const CPU_FREQ_HZ: u32 = 240_000_000;
    
    /// 系统 Tick 频率 (Hz) - Embassy 配置为 1MHz
    pub const TICK_FREQ_HZ: u32 = 1_000_000;
    
    /// 高优先级中断等级
    pub const HIGH_PRIORITY: u8 = 7;
    
    /// 中优先级中断等级
    pub const MID_PRIORITY: u8 = 5;
    
    /// 低优先级中断等级
    pub const LOW_PRIORITY: u8 = 3;
    
    /// 核间通信中断等级 (预留)
    pub const IPC_PRIORITY: u8 = 6;
    
    /// 默认任务栈大小 (字节)
    pub const DEFAULT_STACK_SIZE: usize = 4096;
    
    /// 最小任务栈大小 (字节)
    pub const MIN_STACK_SIZE: usize = 512;
    
    /// 环形缓冲区默认大小
    pub const DEFAULT_RINGBUF_SIZE: usize = 256;
}

/// 内存区域标记
pub mod mem {
    /// 标记数据应放入 DRAM (快速内部 RAM)
    /// 用于频繁访问的数据
    #[macro_export]
    macro_rules! dram_data {
        ($item:item) => {
            #[link_section = ".dram.data"]
            $item
        };
    }
    
    /// 标记数据应放入 IRAM (指令 RAM)
    /// 用于中断处理函数
    #[macro_export]
    macro_rules! iram_text {
        ($item:item) => {
            #[link_section = ".iram.text"]
            $item
        };
    }
    
    /// 标记数据应放入 PSRAM (外部 RAM)
    /// 仅用于大型非关键数据缓冲区
    #[macro_export]
    macro_rules! psram_data {
        ($item:item) => {
            #[link_section = ".psram.data"]
            $item
        };
    }
}
