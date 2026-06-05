use crate::models::ctx::{UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};

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
}

/// **Export dylib plugin**
pub type FnGetDylibPlugin = extern "C" fn() -> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>);