mod config;
mod pipeline;

use config::init_logging;
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_plugins::input::default_input_handler::DefaultInputHandler;
use file_uploader_plugins::pre_upload::upload_file_validator::UploadFileValidator;
use file_uploader_sdk::models::ctx::UploadInputCtx;
use std::sync::Arc;
use tracing::{error, info};

fn main() {
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }

    let ctx = UploadInputCtx {
        file: None,
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
        work_dir: None,
    };

    let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug");

    info!("=== Testing IN-PROCESS plugins ===");
    let input_dir = target_dir.join("resources/input/default_input_handler");
    let Ok(input_plugin) = UploadPluginInfo::new_in_process(
        input_dir.to_str().unwrap(),
        Box::new(DefaultInputHandler),
    ) else {
        error!("Input plugin load error");
        return;
    };
    info!("Input plugin loaded: {}", input_plugin.id);

    let validator_dir = target_dir.join("resources/pre/upload_file_validator");
    let Ok(validator_plugin) = UploadPluginInfo::new_in_process(
        validator_dir.to_str().unwrap(),
        Box::new(UploadFileValidator),
    ) else {
        error!("Validator plugin load error");
        return;
    };
    info!("Validator plugin loaded: {}", validator_plugin.id);

    let in_process_dir = validator_dir.clone();
    let plugin = validator_plugin;
    info!("Plugin loaded: {}", plugin.id);
    let result = match plugin.slot.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            error!("Plugin execute error: {:?}", e);
            return;
        }
    };
    info!(
        "Plugin execute result: {:?}",
        serde_json::to_string(&result).unwrap_or("plugin error".to_string())
    );

    info!("=== Testing DYLIB plugin WITH logger ===");
    let dylib_path = target_dir.join("libuploader_example_plugin.dylib");
    let Ok(dylib_plugin) = UploadPluginInfo::new_from_dylib_path(dylib_path.to_str().unwrap())
    else {
        error!("Dylib plugin load error");
        return;
    };
    info!("Dylib plugin loaded: {}", dylib_plugin.id);
    let result = match dylib_plugin.slot.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            error!("Dylib plugin execute error: {:?}", e);
            return;
        }
    };
    info!(
        "Dylib plugin execute result: {:?}",
        serde_json::to_string(&result).unwrap_or("plugin error".to_string())
    );
}
