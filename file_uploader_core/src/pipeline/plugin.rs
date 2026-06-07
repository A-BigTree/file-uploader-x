use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{UploadConfigType, UploadPhase};
use file_uploader_sdk::models::interface::{UploadDylibPlugin, UploadDylibPluginDyn, UploadPlugin};
use file_uploader_sdk::utils::ctx_util::{convert_input_ctx_s, convert_output_ctx};
use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stabby::boxed::Box as SBox;
use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;
use tracing::error;

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
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
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
    fn on_load(&self) {
        match self {
            PluginSlot::InProcess(plugin) => plugin.on_load(),
            PluginSlot::Dylib { plugin, .. } => plugin.on_load(),
        }
    }

    /// 卸载插件钩子
    fn on_unload(&self) {
        match self {
            PluginSlot::InProcess(plugin) => plugin.on_unload(),
            PluginSlot::Dylib { plugin, .. } => plugin.on_unload(),
        }
    }
}

/// **插件元数据**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginMeta {
    // 插件名称
    pub name: String,
    // 插件版本
    pub version: String,
    // 插件描述
    pub description: String,
    // 插件作者
    pub author: Option<String>,
    // 插件执行阶段
    pub phase: UploadPhase,
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
    pub slot: Arc<PluginSlot>,
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
        let slot = Arc::new(PluginSlot::InProcess(Arc::from(plugin)));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::ctx::UploadInputCtx;
    use file_uploader_sdk::models::interface::UploadPlugin;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_plugin_slot_execute_in_process() {
        todo!("测试 InProcess 插件的 execute 方法")
    }
}
