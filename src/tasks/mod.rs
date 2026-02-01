//! 任务模块
//!
//! 提供不同优先级的任务实现:
//! - `critical`: 高优先级实时任务 (IRAM 执行)
//! - `normal`: 普通优先级任务
//! - `multicore`: 双核调度支持

pub mod critical;
pub mod normal;
pub mod multicore;
