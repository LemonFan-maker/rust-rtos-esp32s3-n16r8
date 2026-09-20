//! 文件系统模块
//! 基于littlefs2实现的嵌入式文件系统支持，特性：
//! - 掉电安全的日志结构文件系统
//! - 支持ESP32分区表
//! - 可配置的文件系统大小和块大小
//! - 目录和文件操作API

pub mod littlefs;
pub mod partition;
pub mod storage;

pub use littlefs::{FileSystem, File, Dir, OpenOptions, FileType, Metadata};
pub use partition::{PartitionTable, Partition, PartitionType, DataSubType, AppSubType};
pub use storage::{FlashStorage, StorageError};
