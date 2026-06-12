use crate::models::interface::PluginLogCallback;
use std::sync::OnceLock;

pub static LOGGER_CALLBACK: OnceLock<PluginLogCallback> = OnceLock::new();

pub fn set_logger_callback(callback: PluginLogCallback) {
    let _ = LOGGER_CALLBACK.set(callback);
}

pub fn is_logger_set() -> bool {
    LOGGER_CALLBACK.get().is_some()
}

#[macro_export]
macro_rules! plugin_log {
    ($level:expr, $($arg:tt)*) => {{
        #[warn(plugin_logger_not_set)]
        if let Some(callback) = $crate::logger::LOGGER_CALLBACK.get() {
            let message = format!($($arg)*);
            callback($level, message.into());
        } else {
            let message = format!($($arg)*);
            eprintln!("[PLUGIN LOG WARNING] Logger callback not set: {}", message);
        }
    }};
}

#[macro_export]
macro_rules! plugin_trace {
    ($($arg:tt)*) => {{
        $crate::plugin_log!($crate::models::enums::PluginLogLevel::Trace, $($arg)*);
    }};
}

#[macro_export]
macro_rules! plugin_debug {
    ($($arg:tt)*) => {{
        $crate::plugin_log!($crate::models::enums::PluginLogLevel::Debug, $($arg)*);
    }};
}

#[macro_export]
macro_rules! plugin_info {
    ($($arg:tt)*) => {{
        $crate::plugin_log!($crate::models::enums::PluginLogLevel::Info, $($arg)*);
    }};
}

#[macro_export]
macro_rules! plugin_warn {
    ($($arg:tt)*) => {{
        $crate::plugin_log!($crate::models::enums::PluginLogLevel::Warn, $($arg)*);
    }};
}

#[macro_export]
macro_rules! plugin_error {
    ($($arg:tt)*) => {{
        $crate::plugin_log!($crate::models::enums::PluginLogLevel::Error, $($arg)*);
    }};
}
