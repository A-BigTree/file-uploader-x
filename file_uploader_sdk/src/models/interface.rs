use crate::models::ctx::{UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};
use crate::models::enums::{PluginLogLevel, UploadPhase};
use stabby::string::String as SString;

/// **Plugin in process**
pub trait UploadPlugin: Send + Sync + 'static {
    /// Plugin name in process
    fn name(&self) -> &'static str;
    /// Plugin phase
    fn phase(&self) -> UploadPhase;
    /// Execute the plugin
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx;
    /// Called when the plugin is loaded
    fn on_load(&self) {}
    /// Called when the plugin is unloaded
    fn on_unload(&self) {}

    /// **入参校验**
    ///
    /// 在框架声明式约束（config.json 的 required / 长度 / 正则 / 范围等）
    /// 校验通过之后调用，用于插件自身的业务级校验。
    ///
    /// 只需读取 `ctx.config_info`（可配合 `utils::config_util` 的 `get_*` 系列）。
    /// 返回 `Err(msg)` 表示校验失败，默认实现直接通过。
    fn validate_params(&self, _ctx: &UploadInputCtx) -> Result<(), String> {
        Ok(())
    }
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
    extern "C" fn set_logger(&self, _callback: PluginLogCallback) {}

    /// **入参校验**
    ///
    /// 在框架声明式约束校验通过之后调用，用于插件自身的业务级校验。
    /// 返回 `Some(msg)` 表示校验失败，`None` 表示通过；默认实现直接通过。
    ///
    /// # Example
    /// ```ignore
    /// extern "C" fn validate_params(&self, ctx: &UploadInputCtxS)
    ///     -> stabby::option::Option<SString> {
    ///     let ctx = convert_input_ctx(ctx);
    ///     match config_util::get_group(&ctx.config_info).as_deref() {
    ///         Some("oss") => stabby::option::Option::None(),
    ///         _ => stabby::option::Option::Some("未知分组".to_string().into()),
    ///     }
    /// }
    /// ```
    extern "C" fn validate_params(
        &self,
        _ctx: &UploadInputCtxS,
    ) -> stabby::option::Option<SString> {
        stabby::option::Option::None()
    }
}

/// **Export dylib plugin**
pub type FnGetDylibPlugin =
    extern "C" fn() -> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>);

/// **Plugin log callback type**
pub type PluginLogCallback = extern "C" fn(level: PluginLogLevel, message: SString);
