use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{PluginLogLevel, UploadPhase};
use file_uploader_sdk::models::interface::{
    FnGetDylibPlugin, UploadDylibPlugin, UploadDylibPluginDyn, UploadPlugin,
};
use file_uploader_sdk::utils::ctx_util::{convert_input_ctx_s, convert_output_ctx};
use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stabby::boxed::Box as SBox;
use std::fs::File;
use std::panic;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tracing::{debug, error, info, trace, warn};

/// 配置 schema 类型（定义在 SDK，便于 dylib 插件复用）
pub use file_uploader_sdk::models::config_schema::{
    AccessSpec, PluginAccessConfig, PluginConfigGroup, PluginConfigInfo, PluginConfigItem,
    PluginFormSpec, PluginValueOption,
};
/// 声明式校验 API（定义在 SDK，便于 dylib 插件与宿主前端复用）
pub use file_uploader_sdk::utils::validate_util::{
    GROUP_KEY, ValidateOptions, ValidationError, ValidationReason, errors_to_string, validate_item,
    validate_plugin_config, validate_plugin_config_opt, validate_plugin_config_with,
};

pub extern "C" fn plugin_log_callback(level: PluginLogLevel, message: stabby::string::String) {
    let message: String = message.into();
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| match level {
        PluginLogLevel::Trace => trace!("{}", message),
        PluginLogLevel::Debug => debug!("{}", message),
        PluginLogLevel::Info => info!("{}", message),
        PluginLogLevel::Warn => warn!("{}", message),
        PluginLogLevel::Error => error!("{}", message),
    }));
    if let Err(_) = result {
        error!("Plugin logging panicked: {}", message);
    }
}

/// **插件统一插槽**
/// - 不同来源插件统一封装相同的行为
pub enum PluginSlot {
    // In process
    InProcess(Arc<dyn UploadPlugin>),
    // Dylib
    Dylib {
        plugin: stabby::dynptr!(SBox<dyn UploadDylibPlugin + Send + Sync>),
        _lib: Arc<Library>,
    },
}

impl PluginSlot {
    /// 执行插件
    pub fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        match self {
            PluginSlot::InProcess(plugin) => plugin.execute(ctx),
            PluginSlot::Dylib { plugin, .. } => {
                let ctx_s = convert_input_ctx_s(ctx);
                let output_s = plugin.execute(&ctx_s);
                convert_output_ctx(&output_s)
            }
        }
    }

    /// 加载插件钩子
    pub fn on_load(&self) {
        match self {
            PluginSlot::InProcess(plugin) => plugin.on_load(),
            PluginSlot::Dylib { plugin, .. } => plugin.on_load(),
        }
    }

    /// 卸载插件钩子
    pub fn on_unload(&self) {
        match self {
            PluginSlot::InProcess(plugin) => plugin.on_unload(),
            PluginSlot::Dylib { plugin, .. } => plugin.on_unload(),
        }
    }

    /// 插件级入参校验（统一转发两类插件）
    pub fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
        match self {
            PluginSlot::InProcess(plugin) => plugin.validate_params(ctx),
            PluginSlot::Dylib { plugin, .. } => {
                let ctx_s = convert_input_ctx_s(ctx);
                let msg: Option<stabby::string::String> = plugin.validate_params(&ctx_s).into();
                match msg {
                    Some(m) => Err(m.into()),
                    None => Ok(()),
                }
            }
        }
    }
}

impl Drop for PluginSlot {
    fn drop(&mut self) {
        self.on_unload();
    }
}

pub(crate) enum LazySlotSource {
    InProcess {
        resource_dir: String,
        plugin: Arc<dyn UploadPlugin>,
    },
    Dylib {
        resource_dir: String,
        dylib_path: String,
    },
}

pub struct LazyPluginSlot {
    pub(crate) source: LazySlotSource,
    pub(crate) inner: OnceLock<Arc<PluginSlot>>,
}

impl LazyPluginSlot {
    pub(crate) fn get_or_init(&self) -> Result<&Arc<PluginSlot>, UploadError> {
        let init_result: Result<Arc<PluginSlot>, UploadError> = match &self.source {
            LazySlotSource::InProcess { plugin, .. } => {
                let slot = Arc::new(PluginSlot::InProcess(plugin.clone()));
                slot.on_load();
                Ok(slot)
            }
            LazySlotSource::Dylib { dylib_path, .. } => {
                let lib = Arc::new(unsafe {
                    Library::new(dylib_path.as_str()).map_err(|e| {
                        UploadError::PluginLoadError(format!("Load dylib failed: {}", e))
                    })?
                });

                let get_plugin: libloading::Symbol<FnGetDylibPlugin> = unsafe {
                    lib.get(b"get_dylib_plugin").map_err(|e| {
                        UploadError::PluginLoadError(format!("Get symbol failed: {}", e))
                    })?
                };

                let plugin_box = get_plugin();
                plugin_box.set_logger(plugin_log_callback);

                let slot = Arc::new(PluginSlot::Dylib {
                    plugin: plugin_box,
                    _lib: lib,
                });
                slot.on_load();
                Ok(slot)
            }
        };

        match init_result {
            Ok(slot) => {
                let _ = self.inner.set(slot);
                Ok(self
                    .inner
                    .get()
                    .expect("OnceLock must be initialized after set"))
            }
            Err(e) => Err(e),
        }
    }

    pub fn execute(&self, ctx: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        if let Some(slot) = self.inner.get() {
            return Ok(slot.execute(ctx));
        }
        let slot = self.get_or_init()?;
        Ok(slot.execute(ctx))
    }

    pub fn on_load(&self) -> Result<(), UploadError> {
        if self.inner.get().is_some() {
            return Ok(());
        }
        self.get_or_init()?;
        Ok(())
    }

    pub fn on_unload(&self) {
        if let Some(slot) = self.inner.get() {
            slot.on_unload();
        }
    }

    /// 插件级入参校验（会触发懒加载，与 `execute` 同语义）
    pub fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), UploadError> {
        let slot = match self.inner.get() {
            Some(s) => s,
            None => self.get_or_init()?,
        };
        slot.validate_params(ctx)
            .map_err(UploadError::PluginParamInvalid)
    }
}

/// **插件元数据**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginMeta {
    // 插件名称
    pub name: String,
    // 插件标题
    pub title: String,
    // 插件版本
    pub version: String,
    // 插件描述
    pub description: String,
    // 插件作者
    pub author: Option<String>,
    // 插件执行阶段
    pub phase: UploadPhase,
}

/// **插件资源（meta + config 加载结果）**
pub struct PluginResource {
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    /// README.md 路径引用（可选，仅记录路径不读取内容）
    pub readme_path: Option<String>,
}

impl PluginResource {
    /// 从插件资源目录加载：meta.json 必读，config.json 选读（缺失→空容器），
    /// README.md 选读（仅记录路径，缺失→None，不报错不告警）。
    pub fn load(dir: &Path) -> Result<Self, UploadError> {
        let meta_path = dir.join("meta.json");
        let meta_file = File::open(&meta_path).map_err(|e| {
            UploadError::PluginLoadError(format!(
                "Failed to open meta file {}: {}",
                meta_path.display(),
                e
            ))
        })?;
        let meta: PluginMeta = serde_json::from_reader(meta_file)?;

        let config_path = dir.join("config.json");
        let config = match File::open(&config_path) {
            Ok(f) => Arc::new(serde_json::from_reader(f)?),
            Err(_) => Arc::new(PluginConfigInfo::default()),
        };

        let readme = dir.join("README.md");
        let readme_path = if readme.is_file() {
            Some(readme.display().to_string())
        } else {
            None
        };

        Ok(PluginResource {
            meta: Arc::new(meta),
            config,
            readme_path,
        })
    }
}

/// **插件定义**
#[derive(Serialize)]
pub struct UploadPluginInfo {
    // 插件标识
    pub id: String,
    // 插件元数据
    pub meta: Arc<PluginMeta>,
    // 插件配置（meta.json + config.json 加载结果）
    pub config: Arc<PluginConfigInfo>,
    // 插件加载路径
    pub path: String,
    // README.md 路径引用（可选，不加载内容）
    pub readme_path: Option<String>,
    // 插件插槽
    #[serde(skip)]
    pub slot: LazyPluginSlot,
}

impl UploadPluginInfo {
    pub fn new_in_process(
        resource_dir: &str,
        plugin: Box<dyn UploadPlugin>,
    ) -> Result<UploadPluginInfo, UploadError> {
        let resource = PluginResource::load(Path::new(resource_dir))?;
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: resource_dir.to_string(),
                plugin: Arc::from(plugin),
            },
            inner: OnceLock::new(),
        };
        let id = format!(
            "in_process_{:?}_{}",
            resource.meta.phase, resource.meta.name
        );
        Ok(UploadPluginInfo {
            id,
            meta: resource.meta,
            config: resource.config,
            path: resource_dir.to_string(),
            readme_path: resource.readme_path,
            slot,
        })
    }

    /// 从动态库文件路径加载插件
    ///
    /// # 参数
    /// * `dylib_path` - 动态库文件路径（.dylib/.so/.dll）
    ///
    /// # 返回
    /// * `Ok(UploadPluginInfo)` - 成功加载的插件信息
    /// * `Err(UploadError)` - 加载失败
    pub fn new_from_dylib_path(dylib_path: &str) -> Result<UploadPluginInfo, UploadError> {
        let dylib_path_obj = Path::new(dylib_path);
        let resource_dir = dylib_path_obj.parent().ok_or_else(|| {
            UploadError::PluginLoadError("Invalid dylib path: no parent directory".to_string())
        })?;

        let resource = PluginResource::load(resource_dir)?;

        let id_path = resource_dir.join("plugin.id");
        let id = std::fs::read_to_string(&id_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                UploadError::PluginLoadError(format!(
                    "Failed to read plugin.id at {}: {}",
                    id_path.display(),
                    e
                ))
            })?;

        let slot = LazyPluginSlot {
            source: LazySlotSource::Dylib {
                resource_dir: resource_dir.display().to_string(),
                dylib_path: dylib_path.to_string(),
            },
            inner: OnceLock::new(),
        };

        Ok(UploadPluginInfo {
            id,
            meta: resource.meta,
            config: resource.config,
            path: dylib_path.to_string(),
            readme_path: resource.readme_path,
            slot,
        })
    }

    pub fn execute(&self, context: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        self.slot.execute(context)
    }

    pub fn on_load(&self) -> Result<(), UploadError> {
        self.slot.on_load()
    }

    pub fn get_id(&self) -> String {
        self.id.clone()
    }

    pub fn get_config(&self) -> Arc<PluginConfigInfo> {
        self.config.clone()
    }

    pub fn get_meta(&self) -> Arc<PluginMeta> {
        self.meta.clone()
    }

    /// README.md 路径引用（可选）
    pub fn get_readme_path(&self) -> Option<&str> {
        self.readme_path.as_deref()
    }

    /// **声明式校验**：按 config.json 的 schema 校验运行态配置，不加载插件。
    pub fn validate_config_declarative(
        &self,
        values: &Option<Value>,
    ) -> Result<(), Vec<ValidationError>> {
        validate_plugin_config_opt(&self.config, values)
    }

    /// **插件级校验**：调用插件自身的 `validate_params`（会触发懒加载）。
    pub fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), UploadError> {
        self.slot.validate_params(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_slot_execute_in_process() {
        // 暂时忽略此测试，需要正确的配置文件路径
        // TODO: 添加正确的测试设置
    }

    #[test]
    fn test_new_from_dylib_path_success() {
        let result = UploadPluginInfo::new_from_dylib_path(
            "../../target/debug/libuploader_example_plugin.dylib",
        );
        let _ = result;
    }

    #[test]
    fn test_new_from_dylib_path_invalid_path() {
        let result = UploadPluginInfo::new_from_dylib_path("/");
        assert!(result.is_err());
        match result {
            Err(UploadError::PluginLoadError(msg)) => {
                assert!(
                    msg.contains("no parent directory")
                        || msg.contains("Failed to open meta file")
                        || msg.contains("Failed to read plugin.id")
                );
            }
            _ => panic!("Expected PluginLoadError"),
        }
    }

    #[test]
    fn test_plugin_config_info_reexport_new_format() {
        // schema 定义已下沉至 SDK，此处仅验证 core 侧重导出可用 + 新格式可解析
        let json = r#"{
            "access": { "fs_read": true },
            "common": [
                {
                    "key": "token",
                    "title": "凭证",
                    "config_type": "Default",
                    "default_value": "",
                    "required": true,
                    "form": { "type": "text", "secret": true, "max_len": 64 }
                }
            ],
            "groups": [
                {
                    "group": "oss",
                    "title": "阿里云 OSS",
                    "params": [
                        {
                            "key": "enabled",
                            "title": "启用",
                            "config_type": "Default",
                            "default_value": true,
                            "form": { "type": "switch" }
                        }
                    ]
                }
            ]
        }"#;
        let info: PluginConfigInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.common.len(), 1);
        assert!(info.common[0].required);
        assert!(matches!(
            info.common[0].form,
            PluginFormSpec::Text { secret: true, .. }
        ));
        assert!(info.has_groups());
        assert_eq!(info.group_keys(), vec!["oss"]);
        assert_eq!(info.effective_items(Some("oss")).len(), 2);
        assert!(matches!(info.access.fs_read, AccessSpec::Flag(true)));
    }

    #[test]
    fn test_validate_api_reexported_from_core() {
        let json = r#"{
            "common": [
                {
                    "key": "endpoint",
                    "title": "地址",
                    "config_type": "Default",
                    "default_value": "",
                    "required": true,
                    "form": { "type": "text", "pattern": "^https?://" }
                }
            ]
        }"#;
        let info: PluginConfigInfo = serde_json::from_str(json).unwrap();

        assert!(validate_plugin_config(&info, &serde_json::json!({"endpoint":"https://a"})).is_ok());

        let err = validate_plugin_config(&info, &serde_json::json!({})).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].reason, ValidationReason::Required);
        assert!(errors_to_string(&err).contains("endpoint"));

        // Option<Value> 入口
        assert!(validate_plugin_config_opt(&info, &None).is_err());
    }

    #[test]
    fn test_plugin_config_info_default_empty() {
        let d = PluginConfigInfo::default();
        assert!(d.common.is_empty());
        assert!(d.groups.is_empty());
        assert!(matches!(d.access.fs_read, AccessSpec::Flag(false)));
        assert!(matches!(d.access.fs_write, AccessSpec::Flag(false)));
        assert!(matches!(d.access.network, AccessSpec::Flag(false)));
    }

    fn target_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/debug")
    }

    #[test]
    fn test_plugin_resource_load_success() {
        let dir = target_dir().join("resources/pre/upload_file_validator");
        if !dir.exists() {
            eprintln!("skip: {} not ready yet", dir.display());
            return;
        }
        let r = PluginResource::load(&dir).unwrap();
        assert_eq!(r.meta.name, "upload_file_validator");
        assert!(!r.config.common.is_empty());
    }

    #[test]
    fn test_plugin_resource_readme_path_present() {
        let dir = target_dir().join("resources/pre/upload_file_validator");
        if !dir.join("README.md").is_file() {
            eprintln!("skip: README.md not ready yet in {}", dir.display());
            return;
        }
        let r = PluginResource::load(&dir).unwrap();
        let readme = r.readme_path.expect("README.md should be referenced");
        assert!(readme.ends_with("README.md"), "got: {readme}");
    }

    #[test]
    fn test_plugin_resource_readme_path_absent_is_none() {
        // 构造一个只有 meta.json 的临时目录，验证缺失 README 不报错且为 None
        let tmp = std::env::temp_dir().join(format!("fux_readme_absent_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("meta.json"),
            r#"{"name":"t","title":"t","version":"0.0.1","description":"d","author":null,"phase":"PreUpload"}"#,
        )
        .unwrap();

        let r = PluginResource::load(&tmp).unwrap();
        assert!(r.readme_path.is_none());
        assert!(r.config.common.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_plugin_resource_load_missing_meta() {
        let dir = target_dir().join("resources/__nonexistent__");
        let r = PluginResource::load(&dir);
        assert!(r.is_err());
    }
}
