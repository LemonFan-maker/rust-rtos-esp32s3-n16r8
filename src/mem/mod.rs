//! 内存管理模块
//!
//! 提供以下功能:
//! - PSRAM 初始化与分配 (自动缓存策略)
//! - 内存池分配器 (零拷贝、无锁)
//! - DMA 缓冲区管理 (对齐、cache 一致性)
//!
//! # 内存区域
//!
//! ESP32-S3-N16R8 内存布局:
//! - IRAM: 64KB - 仅关键代码
//! - DRAM: 256KB - 主工作内存
//! - PSRAM: 8MB - 大型缓冲区
//!
//! # 示例
//!
//! ```rust,ignore
//! use rustrtos::mem::{psram, pool, dma};
//!
//! // PSRAM 分配
//! let buffer = psram::alloc::<[u8; 4096]>(CacheMode::Auto);
//!
//! // 内存池分配
//! static POOL: MemoryPool<SensorData, 32, Backend::Dram> = MemoryPool::new();
//! let data = POOL.alloc().unwrap();
//!
//! // DMA 缓冲区
//! let dma_buf = DmaBuffer::<1024>::new(DmaStrategy::Auto);
//! ```

#![allow(dead_code)]

pub mod psram;
pub mod pool;
pub mod dma;

// 重导出常用类型
pub use psram::{CacheMode, PsramConfig, PsramBox};
pub use pool::{MemoryPool, PoolBox, Backend};
pub use dma::{DmaBuffer, DmaStrategy};

/// 内存区域标记宏
/// 
/// 用于将数据放入特定内存区域

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
