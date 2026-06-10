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
        "./pre_upload_plugins.json",
        Box::new(FileTypeFilter),
    ) else {
        error!("Plugin load error");
        return;
    };
}
