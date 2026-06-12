//! file_uploader_sdk - Dynamic library plugin SDK for file_uploader_x
//!
//! # Plugin Development
//!
//! ## Logging
//!
//! Dynamic library plugins should use the `plugin_*!` macros (e.g., `plugin_info!`)
//! for logging. These macros require the plugin to implement the `set_logger` method
//! and call `set_logger_callback(callback)` to receive logs from the main program.
//!
//! ### Example
//!
//! ```ignore
//! use file_uploader_sdk::{plugin_info, logger::set_logger_callback};
//! use file_uploader_sdk::models::interface::PluginLogCallback;
//!
//! impl UploadDylibPlugin for MyPlugin {
//!     extern "C" fn set_logger(&self, callback: PluginLogCallback) {
//!         set_logger_callback(callback);
//!     }
//!     
//!     extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS {
//!         plugin_info!("MyPlugin executing...");
//!         // ...
//!     }
//! }
//! ```

pub mod error;
pub mod logger;
pub mod models;
pub mod utils;
