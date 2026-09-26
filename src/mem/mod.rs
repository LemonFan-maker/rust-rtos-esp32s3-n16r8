#![allow(dead_code)]

pub mod psram;
pub mod pool;
pub mod dma;
pub mod gdma;

pub use psram::{PsramConfig, PsramBox};
pub use pool::{MemoryPool, PoolBox, Backend};
pub use dma::DmaBuffer;

#[macro_export]
macro_rules! dram_data {
    ($item:item) => {
        #[link_section = ".dram.data"]
        $item
    };
}

#[macro_export]
macro_rules! iram_text {
    ($item:item) => {
        #[link_section = ".iram.text"]
        $item
    };
}

#[macro_export]
macro_rules! psram_data {
    ($item:item) => {
        #[link_section = ".psram.data"]
        $item
    };
}
