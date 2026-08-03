use file_uploader_sdk::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::PluginLogCallback;
use file_uploader_sdk::models::interface::UploadDylibPlugin;
use file_uploader_sdk::utils::config_util;
use file_uploader_sdk::utils::ctx_util::convert_input_ctx;
use file_uploader_sdk::{logger::set_logger_callback, plugin_info};
use stabby::string::String as SString;

pub struct UploaderTestExamplePlugin;

impl UploadDylibPlugin for UploaderTestExamplePlugin {
    extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS {
        plugin_info!("UploaderTestExamplePlugin execute...");
        let ctx = convert_input_ctx(ctx);
        let json = serde_json::to_string(&ctx).unwrap_or("input error".to_string());
        plugin_info!("UploaderTestExamplePlugin get input: {}", json);
        UploadOutputCtxS {
            result: OutputResultType::Success,
            message: "成功".to_string().into(),
            file: stabby::option::Option::None(),
            extra_info: stabby::option::Option::None(),
        }
    }

    extern "C" fn on_load(&self) {
        plugin_info!("UploaderTestExamplePlugin loading...")
    }

    extern "C" fn on_unload(&self) {
        plugin_info!("UploaderTestExamplePlugin unloading...")
    }

    extern "C" fn set_logger(&self, callback: PluginLogCallback) {
        set_logger_callback(callback);
    }

    /// 业务级入参校验：`group` 只能是 config.json 中声明的 `oss` / `local`
    extern "C" fn validate_params(
        &self,
        ctx: &UploadInputCtxS,
    ) -> stabby::option::Option<SString> {
        let ctx = convert_input_ctx(ctx);
        match config_util::get_group(&ctx.config_info).as_deref() {
            Some(g @ ("oss" | "local")) => {
                plugin_info!("UploaderTestExamplePlugin validate_params ok, group={}", g);
                stabby::option::Option::None()
            }
            Some(other) => stabby::option::Option::Some(
                format!("未知分组 '{other}'，可选值: [oss, local]").into(),
            ),
            None => {
                stabby::option::Option::Some("缺少分组标识 group，可选值: [oss, local]".into())
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_dylib_plugin()
-> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>) {
    stabby::boxed::Box::new(UploaderTestExamplePlugin).into()
}
