//! 条件编译日志系统
//!
//! 根据 feature 选择不同的日志后端:
//! - `log-defmt`: 使用 defmt (高效二进制日志)
//! - `dev` / `log-println`: 使用 esp-println (文本日志)
//! - 默认 (release): 完全禁用日志 (零开销)
//!
//! # 日志级别
//! - `error!`: 错误信息
//! - `warn!`: 警告信息
//! - `info!`: 一般信息
//! - `debug!`: 调试信息
//! - `trace!`: 详细跟踪

// ===================================================================
// defmt 后端 (feature = "log-defmt")
// ===================================================================
#[cfg(feature = "log-defmt")]
pub use defmt::{info, debug, warn, error, trace};

#[cfg(feature = "log-defmt")]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { defmt::info!($($arg)*) };
}

#[cfg(feature = "log-defmt")]
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { defmt::debug!($($arg)*) };
}

#[cfg(feature = "log-defmt")]
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { defmt::warn!($($arg)*) };
}

#[cfg(feature = "log-defmt")]
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { defmt::error!($($arg)*) };
}

#[cfg(feature = "log-defmt")]
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => { defmt::trace!($($arg)*) };
}

// ===================================================================
// esp-println 后端 (feature = "dev" 或 "log-println")
// ===================================================================
#[cfg(all(any(feature = "dev", feature = "log-println"), not(feature = "log-defmt")))]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { esp_println::println!("[INFO] {}", format_args!($($arg)*)) };
}

#[cfg(all(any(feature = "dev", feature = "log-println"), not(feature = "log-defmt")))]
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { esp_println::println!("[DEBUG] {}", format_args!($($arg)*)) };
}

#[cfg(all(any(feature = "dev", feature = "log-println"), not(feature = "log-defmt")))]
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { esp_println::println!("[WARN] {}", format_args!($($arg)*)) };
}

#[cfg(all(any(feature = "dev", feature = "log-println"), not(feature = "log-defmt")))]
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { esp_println::println!("[ERROR] {}", format_args!($($arg)*)) };
}

#[cfg(all(any(feature = "dev", feature = "log-println"), not(feature = "log-defmt")))]
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => { esp_println::println!("[TRACE] {}", format_args!($($arg)*)) };
}

// ===================================================================
// 空实现 (release 模式，无日志 feature)
// ===================================================================
#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {};
}

// ===================================================================
// 便捷重导出
// ===================================================================
pub use log_info;
pub use log_debug;
pub use log_warn;
pub use log_error;
pub use log_trace;

// ===================================================================
// 性能计时宏 (仅在 dev 模式下有效)
// ===================================================================

/// 测量代码块执行时间 (仅 dev 模式)
///
/// # Example
/// ```ignore
/// let result = timed!("heavy_computation", {
///     heavy_computation()
/// });
/// // 输出: [TIME] heavy_computation: 1234μs
/// ```
#[cfg(any(feature = "dev", feature = "log-defmt"))]
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:expr) => {{
        let start = embassy_time::Instant::now();
        let result = $block;
        let elapsed = start.elapsed().as_micros();
        defmt::info!("[TIME] {}: {}μs", $name, elapsed);
        result
    }};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt")))]
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:expr) => {
        $block
    };
}

pub use timed;

// ===================================================================
// 断言宏 (release 模式下可配置)
// ===================================================================

/// Debug 断言 (仅在 debug 模式下检查)
#[macro_export]
macro_rules! debug_assert_msg {
    ($cond:expr, $($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            if !$cond {
                $crate::log_error!("Assertion failed: {}", format_args!($($arg)*));
                panic!("Assertion failed");
            }
        }
    };
}

pub use debug_assert_msg;
