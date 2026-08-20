mod config;
mod pipeline;

use config::init_logging;
use file_uploader_core::pipeline::plugin::{
    UploadPluginInfo, errors_to_string, validate_plugin_config,
};
use file_uploader_sdk::models::ctx::UploadInputCtx;
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::{error, info, warn};

/// 打印插件的资源引用与配置 schema 概览
fn describe_plugin(plugin: &UploadPluginInfo) {
    info!(
        "plugin '{}' loaded | path={} | readme={}",
        plugin.id,
        plugin.path,
        plugin.get_readme_path().unwrap_or("<none>")
    );
    let config = plugin.get_config();
    info!(
        "  schema: common={} params, groups={:?}",
        config.common.len(),
        config.group_keys()
    );
}

/// 用插件 schema 校验一份运行态配置，打印结果
fn check_config(plugin: &UploadPluginInfo, label: &str, values: &Value) {
    match validate_plugin_config(&plugin.get_config(), values) {
        Ok(()) => info!("  [{}] config OK for '{}'", label, plugin.id),
        Err(errs) => warn!(
            "  [{}] config INVALID for '{}': {}",
            label,
            plugin.id,
            errors_to_string(&errs)
        ),
    }
}

fn main() {
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }

    let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug");

    let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug");

    // ==================== 动态库插件（分组配置） ====================
    info!("=== Testing DYLIB plugin WITH logger ===");
    let dylib_path = target_dir.join("libuploader_example_plugin.dylib");
    let Ok(dylib_plugin) = UploadPluginInfo::new_from_dylib_path(dylib_path.to_str().unwrap())
    else {
        error!("Dylib plugin load error");
        return;
    };
    describe_plugin(&dylib_plugin);

    // 分组型插件：缺 group / 未知 group 都会被声明式校验拦截
    check_config(&dylib_plugin, "missing-group", &json!({}));
    check_config(&dylib_plugin, "unknown-group", &json!({ "group": "s3" }));
    // 激活 oss 但缺必填项
    check_config(&dylib_plugin, "oss-incomplete", &json!({ "group": "oss" }));

    let oss_config = json!({
        "group": "oss",
        "pass_type": ["image/*"],
        "retry_times": 3,
        "endpoint": "https://oss-cn-hangzhou.aliyuncs.com",
        "bucket": "my-bucket",
        "access_key": "AK-demo",
        "access_secret": "SK-demo",
        "use_https": true
    });
    check_config(&dylib_plugin, "oss-valid", &oss_config);

    let dylib_ctx = UploadInputCtx {
        file: None,
        config_info: Arc::new(Some(oss_config)),
        extra_info: None,
        work_dir: None,
    };

    // dylib 插件级校验（跨 stabby ABI 转发）
    match dylib_plugin.validate_params(&dylib_ctx) {
        Ok(()) => info!("  [plugin-level] dylib validate_params OK"),
        Err(e) => warn!("  [plugin-level] dylib validate_params failed: {}", e),
    }
    // 未知分组会被插件自身拒绝
    let bad_ctx = UploadInputCtx {
        file: None,
        config_info: Arc::new(Some(json!({ "group": "s3" }))),
        extra_info: None,
        work_dir: None,
    };
    match dylib_plugin.validate_params(&bad_ctx) {
        Ok(()) => warn!("  [plugin-level] dylib unexpectedly accepted group=s3"),
        Err(e) => info!("  [plugin-level] dylib rejected group=s3 as expected: {}", e),
    }

    let result = match dylib_plugin.slot.execute(&dylib_ctx) {
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
