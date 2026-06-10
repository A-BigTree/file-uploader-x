mod config;
mod pipeline;

use config::init_logging;
use tracing::{error, info};
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_plugins::pre_upload::file_type_filter::FileTypeFilter;
use file_uploader_sdk::models::ctx::UploadInputCtx;

fn main() {
    // init_logging;
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }
    // 测试插件执行
    let ctx = UploadInputCtx {
        file_list: vec![],
        config_info: None,
        extra_info: None,
        related_process_info: None
    };
    let Ok(plugin) = UploadPluginInfo::new_in_process(
        "./pre_upload_plugins.json",
        Box::new(FileTypeFilter),
    ) else {
        error!("Plugin load error");
        return;
    };
    info!("Plugin loaded: {}", plugin.id);
    let result = plugin.slot.execute(&ctx);
    info!("Plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
    let Ok(dylib_plugin) = UploadPluginInfo::new_from_dylib_path(
        "./libuploader_example_plugin.dylib"
    ) else {
        error!("Dylib plugin load error");
        return;
    };
    info!("Dylib plugin loaded: {}", dylib_plugin.id);
    dylib_plugin.slot.on_load();
    let result = dylib_plugin.slot.execute(&ctx);
    info!("Dylib plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
    dylib_plugin.slot.on_unload();
}
