mod config;
mod pipeline;

use config::init_logging;
use tracing::{error, info};
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_sdk::models::ctx::UploadInputCtx;
use file_uploader_plugins::pre_upload::file_type_filter::FileTypeFilter;

fn main() {
    // init_logging;
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }
    let Ok(plugin) = UploadPluginInfo::new_in_process(
        "./pre_uploader_plugins.json",
        Box::new(FileTypeFilter),
    ) else {
        error!("Plugin load error");
        return;
    };
    info!("Plugin loaded: {}", serde_json::to_string(&plugin).unwrap_or("plugin error".to_string()));
    // 测试插件加载
    plugin.slot.on_load();
    // 测试插件执行
    let ctx = UploadInputCtx {
        file_list: vec![],
        config_info: None,
        extra_info: None,
        related_process_info: None
    };
    let output = plugin.slot.execute(&ctx);
    info!("Plugin execute result: {}", serde_json::to_string(&output).unwrap_or("plugin error".to_string()));
    // 测试插件卸载
    plugin.slot.on_unload();
}
