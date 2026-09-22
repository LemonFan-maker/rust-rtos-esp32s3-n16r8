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

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

#[cfg(not(any(feature = "dev", feature = "log-defmt", feature = "log-println")))]
#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {{
        fn _log_disabled(_: ::core::fmt::Arguments<'_>) {}
        _log_disabled(::core::format_args!($($arg)*));
    }};
}

pub use log_info;
pub use log_debug;
pub use log_warn;
pub use log_error;
pub use log_trace;

#[cfg(any(feature = "dev", feature = "log-defmt"))]
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:expr) => {{
        let start = embassy_time::Instant::now();
        let result = $block;
        let elapsed = start.elapsed().as_micros();
        defmt::info!("[TIME] {}: {} us", $name, elapsed);
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
