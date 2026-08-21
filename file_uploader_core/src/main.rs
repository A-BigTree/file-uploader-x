use file_uploader_core::config::init_logging;
use file_uploader_core::pipeline::callback::{PipelineCallback, PipelineEvent};
use file_uploader_core::pipeline::plugin::{
    UploadPluginInfo, errors_to_string, validate_plugin_config,
};
use file_uploader_core::pipeline::registry::{
    PluginRegistryInfo, PluginRegistryStatus, UploadPluginRegistryTable,
};
use file_uploader_core::pipeline::stage::{StageExecutionContext, StageExecute};
use file_uploader_core::{register_in_process_plugins, InProcessEntry};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::{error, info, warn};

/// 演示用进程内插件：Input 阶段直通（无文件时返回 Success 空输出）。
struct PassthroughInput;

impl UploadPlugin for PassthroughInput {
    fn name(&self) -> &'static str {
        "passthrough_input"
    }
    fn phase(&self) -> UploadPhase {
        UploadPhase::Input
    }
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        UploadOutputCtx {
            result: OutputResultType::Success,
            message: "passthrough".into(),
            file: ctx.file.clone(),
            extra_info: None,
        }
    }
}

/// 演示用事件回调：打印事件。
struct PrintCallback;

impl PipelineCallback for PrintCallback {
    fn on_event(&self, event: &PipelineEvent, _ctx: &UploadInputCtx, _result: Option<&UploadOutputCtx>) {
        info!("[event] {:?} phase={:?}", event.kind, event.phase);
    }
}

/// 演示用阶段编排回调：接管阶段执行（示例：直接跳过全部插件，产出成功输出）。
struct SkipAllExecutor;

impl StageExecute for SkipAllExecutor {
    fn execute(&self, ctx: &mut StageExecutionContext) -> Result<UploadOutputCtx, UploadError> {
        info!(
            "[stage-executor] phase={:?} plugins={} -> skip all",
            ctx.phase(),
            ctx.plugins().len()
        );
        Ok(UploadOutputCtx {
            result: OutputResultType::Success,
            message: "skipped by custom executor".into(),
            file: ctx.current_input().file.clone(),
            extra_info: ctx.current_input().extra_info.clone(),
        })
    }
}

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

    // ==================== 进程内插件（宿主注入模式） ====================
    info!("=== Testing IN-PROCESS plugins (host-injected) ===");

    // 1. 宿主在临时目录准备插件资源（meta.json 必需）
    let resource_root = std::env::temp_dir().join(format!("fux_demo_{}", std::process::id()));
    let plugin_dir = resource_root.join("input/passthrough_input");
    if let Err(e) = std::fs::create_dir_all(&plugin_dir) {
        error!("create demo resource dir failed: {}", e);
        return;
    }
    if let Err(e) = std::fs::write(
        plugin_dir.join("meta.json"),
        r#"{"name":"passthrough_input","title":"Demo","description":"host-injected demo","version":"0.0.1","author":null,"phase":"Input"}"#,
    ) {
        error!("write demo meta.json failed: {}", e);
        return;
    }

    // 2. 注册清单（这是宿主注入进程内插件的唯一入口）
    let entries = [InProcessEntry {
        resource_subdir: "input/passthrough_input",
        factory: || -> Arc<dyn UploadPlugin> { Arc::new(PassthroughInput) },
    }];
    if let Err(e) = register_in_process_plugins(&entries, &resource_root) {
        error!("register in-process plugins failed: {}", e);
        return;
    }

    // 3. 全局查询 + 装配 RegistryTable（含事件回调与阶段编排回调演示）
    let summaries = file_uploader_core::list_in_process_plugins().expect("list registered");
    info!("registered {} in-process plugin(s)", summaries.len());
    let info = match file_uploader_core::get_in_process_plugin_info(&summaries[0].id) {
        Ok(Some(i)) => i,
        _ => {
            error!("get_in_process_plugin_info failed");
            return;
        }
    };
    describe_plugin(&info);
    check_config(&info, "empty", &json!({}));

    let reg = PluginRegistryInfo::new(info, 1, PluginRegistryStatus::Enable, None);
    let table = UploadPluginRegistryTable::new("demo_registry".into(), vec![reg])
        .with_event_callback(Arc::new(PrintCallback))
        .with_stage_executor(UploadPhase::PreUpload, Arc::new(SkipAllExecutor));

    let input = UploadInputCtx {
        file: None,
        config_info: Arc::new(None),
        extra_info: None,
        work_dir: None,
    };
    let output = table.execute_pipeline(input);
    info!(
        "pipeline result: {:?} message={}",
        output.result, output.message
    );

    let _ = std::fs::remove_dir_all(&resource_root);

    // ==================== 动态库插件（分组配置） ====================
    info!("=== Testing DYLIB plugin WITH logger ===");
    let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug");
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
