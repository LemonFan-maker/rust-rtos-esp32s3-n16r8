//! 主机侧单元测试聚合crate。
//!
//! 以`#[path]`把固件仓库中的纯逻辑模块纳入本包, 在x86主机上直接运行
//! `cargo test`(无需QEMU/硬件)。被测模块本身不依赖esp-hal(硬件相关部分
//! 已用`cfg(target_arch)`隔离), 本包仅做编译聚合。
//!
//! 运行: `cd host-tests && cargo test`

// 被测模块以路径方式纳入, 保留源文件中的#[cfg(test)]测试。
#[path = "../../src/sync/ringbuffer.rs"]
mod ringbuffer;

#[path = "../../src/perf.rs"]
mod perf;

#[path = "../../src/watchdog.rs"]
mod watchdog;
