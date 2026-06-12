use crate::models::ctx::{UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};
use crate::models::enums::PluginLogLevel;
use stabby::string::String as SString;

/// **Plugin in process**
pub trait UploadPlugin: Send + Sync + 'static {
    /// Plugin name in process
    fn name(&self) -> &'static str;
    /// Execute the plugin
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx;
    /// Called when the plugin is loaded
    fn on_load(&self) {}
    /// Called when the plugin is unloaded
    fn on_unload(&self) {}
}

/// **Plugin in dylib**
#[stabby::stabby]
pub trait UploadDylibPlugin: Send + Sync {
    /// Execute the plugin
    extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS;

    /// Called when the plugin is loaded
    extern "C" fn on_load(&self) {}

    /// Called when the plugin is unloaded
    extern "C" fn on_unload(&self) {}

    /// Set logger callback for the plugin
    ///
    /// # IMPORTANT
    /// Plugins MUST override this method and call `set_logger_callback(callback)`
    /// to enable logging. Otherwise, all `plugin_*!` macro calls will be silently ignored.
    ///
    /// # Example
    /// ```ignore
    /// extern "C" fn set_logger(&self, callback: PluginLogCallback) {
    ///     set_logger_callback(callback);
    /// }
    /// ```
    extern "C" fn set_logger(&self, callback: PluginLogCallback) {}
}

/// **Export dylib plugin**
pub type FnGetDylibPlugin =
    extern "C" fn() -> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>);

/// **Plugin log callback type**
pub type PluginLogCallback = extern "C" fn(level: PluginLogLevel, message: SString);
