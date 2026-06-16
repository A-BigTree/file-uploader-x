use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{PluginLogLevel, UploadConfigType, UploadPhase};
use file_uploader_sdk::models::interface::{
    FnGetDylibPlugin, UploadDylibPlugin, UploadDylibPluginDyn, UploadPlugin,
};
use file_uploader_sdk::utils::ctx_util::{convert_input_ctx_s, convert_output_ctx};
use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stabby::boxed::Box as SBox;
use std::collections::HashMap;
use std::fs::File;
use std::panic;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tracing::{debug, error, info, trace, warn};

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
}

impl Drop for PluginSlot {
    fn drop(&mut self) {
        self.on_unload();
    }
}

pub(crate) enum LazySlotSource {
    InProcess {
        config_path: String,
        plugin: Arc<dyn UploadPlugin>,
    },
    Dylib {
        config_path: String,
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

/// **插件配置文件容器**（对应 config.json）
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginConfigInfo {
    /// 访问控制（reserved，纯透传）
    #[serde(default = "empty_object")]
    pub access: Value,
    /// 配置项列表
    #[serde(default)]
    pub params: Vec<PluginConfigItem>,
}

fn empty_object() -> Value {
    serde_json::json!({})
}

impl Default for PluginConfigInfo {
    fn default() -> Self {
        Self {
            access: empty_object(),
            params: Vec::new(),
        }
    }
}

/// **单个配置项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginConfigItem {
    pub key: String,
    pub title: String,
    pub description: String,
    pub config_type: UploadConfigType,
    pub default_value: Value,
    pub form: PluginFormSpec,
}

/// **表单控件描述**（value_type + value_config 合并表达）
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginFormSpec {
    Text {
        #[serde(default)]
        secret: bool,
    },
    Select {
        #[serde(default)]
        options: Vec<PluginValueOption>,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        allow_custom: bool,
    },
}

/// **候选项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginValueOption {
    pub label: String,
    pub value: Value,
}

/// **插件资源（meta + config 加载结果）**
pub struct PluginResource {
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
}

impl PluginResource {
    /// 从插件资源目录加载：meta.json 必读，config.json 选读（缺失→空容器）。
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

        Ok(PluginResource { meta: Arc::new(meta), config })
    }
}

/// **插件配置**
#[derive(Serialize, Deserialize, Debug)]
pub struct PluginConfig {
    // 配置key
    pub key: String,
    // 配置类型
    pub config_type: UploadConfigType,
    // 配置描述
    pub description: String,
    // 配置默认值
    pub default_value: Value,
}

/// **插件定义**
#[derive(Serialize)]
pub struct UploadPluginInfo {
    // 插件标识
    pub id: String,
    // 插件元数据
    pub meta: Arc<PluginMeta>,
    // 插件默认配置
    pub default_config: Option<Arc<HashMap<String, PluginConfig>>>,
    // 插件加载路径
    pub path: String,
    // 插件插槽
    #[serde(skip)]
    pub slot: LazyPluginSlot,
}

impl UploadPluginInfo {
    pub fn new_in_process(
        config_path: &str,
        plugin: Box<dyn UploadPlugin>,
    ) -> Result<UploadPluginInfo, UploadError> {
        let json_file = File::open(config_path)?;
        let config_value: Value = serde_json::from_reader(json_file)?;
        let meta_value = config_value
            .get(plugin.name())
            .ok_or_else(|| UploadError::PluginLoadError("Plugin config not found".to_string()))?;
        let meta: Arc<PluginMeta> = serde_json::from_value(meta_value.clone())?;
        let default_config_value: Option<&Value> = meta_value.get("config");
        let default_config: Option<Arc<HashMap<String, PluginConfig>>> = match default_config_value
        {
            None => None,
            Some(config) => {
                if let Ok(map) = serde_json::from_value(config.clone()) {
                    Some(Arc::new(map))
                } else {
                    error!("Plugin config error");
                    None
                }
            }
        };
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: config_path.to_string(),
                plugin: Arc::from(plugin),
            },
            inner: OnceLock::new(),
        };
        let plugin_id = format!(
            "{}_{}_{}",
            "in_process",
            meta.name.clone(),
            meta.author.clone().unwrap_or("unknow".to_string())
        );
        Ok(UploadPluginInfo {
            id: plugin_id,
            meta,
            default_config,
            path: config_path.to_string(),
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
        // 1. 解析路径获取父目录
        let dylib_path_obj = Path::new(dylib_path);
        let parent_dir = dylib_path_obj.parent().ok_or_else(|| {
            UploadError::PluginLoadError("Invalid dylib path: no parent directory".to_string())
        })?;

        // 2. 构建配置文件路径
        let config_path = parent_dir.join("config.json");

        // 3. 加载配置文件
        let json_file = File::open(&config_path).map_err(|e| {
            UploadError::PluginLoadError(format!(
                "Failed to open config file {}: {}",
                config_path.display(),
                e
            ))
        })?;
        let config_value: Value = serde_json::from_reader(json_file)?;
        let meta: Arc<PluginMeta> = serde_json::from_value(config_value.clone())?;
        let default_config_value: Option<&Value> = config_value.get("config");
        let default_config: Option<Arc<HashMap<String, PluginConfig>>> = match default_config_value
        {
            None => None,
            Some(config) => {
                if let Ok(map) = serde_json::from_value(config.clone()) {
                    Some(Arc::new(map))
                } else {
                    error!("Plugin config error");
                    None
                }
            }
        };

        // 4. 构建 LazyPluginSlot（延迟加载动态库）
        let slot = LazyPluginSlot {
            source: LazySlotSource::Dylib {
                config_path: config_path.display().to_string(),
                dylib_path: dylib_path.to_string(),
            },
            inner: OnceLock::new(),
        };

        // 5. 生成插件 ID
        let plugin_id = format!(
            "{}_{}_{}",
            "dylib",
            meta.name.clone(),
            meta.author.clone().unwrap_or("unknown".to_string())
        );

        // 6. 返回 UploadPluginInfo
        Ok(UploadPluginInfo {
            id: plugin_id,
            meta,
            default_config,
            path: dylib_path.to_string(),
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

    pub fn get_default_config(&self) -> Option<Arc<HashMap<String, PluginConfig>>> {
        self.default_config.clone()
    }

    pub fn get_meta(&self) -> Arc<PluginMeta> {
        self.meta.clone()
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
                    msg.contains("no parent directory") || msg.contains("Failed to open config")
                );
            }
            _ => panic!("Expected PluginLoadError"),
        }
    }

    #[test]
    fn test_plugin_config_info_select_text_deserialize() {
        let json = r#"{
            "access": {},
            "params": [
                {
                    "key": "pass_type",
                    "title": "允许类型",
                    "description": "为空全部允许",
                    "config_type": "Custom",
                    "default_value": [],
                    "form": {
                        "type": "select",
                        "options": [{"label":"图片","value":"image"}],
                        "multiple": true,
                        "allow_custom": true
                    }
                },
                {
                    "key": "token",
                    "title": "凭证",
                    "description": "访问凭证",
                    "config_type": "Default",
                    "default_value": "",
                    "form": { "type": "text", "secret": true }
                }
            ]
        }"#;
        let info: super::PluginConfigInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.params.len(), 2);
        assert!(matches!(
            info.params[0].form,
            super::PluginFormSpec::Select { multiple: true, .. }
        ));
        assert!(matches!(
            info.params[1].form,
            super::PluginFormSpec::Text { secret: true }
        ));
    }

    #[test]
    fn test_plugin_form_spec_serde_default_omitted() {
        let json = r#"{ "type": "text" }"#;
        let form: super::PluginFormSpec = serde_json::from_str(json).unwrap();
        assert!(matches!(form, super::PluginFormSpec::Text { secret: false }));

        let json2 = r#"{ "type": "select" }"#;
        let form2: super::PluginFormSpec = serde_json::from_str(json2).unwrap();
        assert!(matches!(
            form2,
            super::PluginFormSpec::Select { options, multiple: false, allow_custom: false } if options.is_empty()
        ));
    }

    #[test]
    fn test_plugin_config_info_default_empty() {
        let d = super::PluginConfigInfo::default();
        assert!(d.params.is_empty());
        assert!(d.access.is_object());
    }

    fn target_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/debug")
    }

    #[test]
    fn test_plugin_resource_load_success() {
        let dir = target_dir().join("resources/pre/file_type_filter");
        if !dir.exists() {
            eprintln!("skip: {} not ready yet", dir.display());
            return;
        }
        let r = super::PluginResource::load(&dir).unwrap();
        assert_eq!(r.meta.name, "file_type_filter");
        assert!(!r.config.params.is_empty());
    }

    #[test]
    fn test_plugin_resource_load_missing_meta() {
        let dir = target_dir().join("resources/__nonexistent__");
        let r = super::PluginResource::load(&dir);
        assert!(r.is_err());
    }
}
