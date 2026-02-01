//! 同步原语模块
//!
//! 提供线程安全的同步原语，基于 embassy-sync 封装:
//! - `CriticalSignal`: 单值信号量
//! - `CriticalChannel`: MPMC 消息队列
//! - `CriticalMutex`: 异步互斥锁
//! - `RingBuffer`: 零拷贝环形缓冲区

pub mod primitives;
pub mod ringbuffer;

pub use primitives::{CriticalSignal, CriticalChannel, CriticalMutex};
pub use ringbuffer::RingBuffer;
