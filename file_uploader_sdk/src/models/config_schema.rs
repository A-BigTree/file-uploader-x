use crate::models::enums::UploadConfigType;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// **权限粒度**（开关或白名单，纯透传不做执行逻辑）
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum AccessSpec {
    /// 全开(true) / 全关(false)
    Flag(bool),
    /// 白名单：仅允许列出的项（fs 为路径，network 为 host）
    Allowlist(Vec<String>),
}

impl Default for AccessSpec {
    fn default() -> Self {
        Self::Flag(false)
    }
}

/// **插件权限配置**（对应 config.json 的 access 字段，纯透传）
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginAccessConfig {
    #[serde(default)]
    pub fs_read: AccessSpec,
    #[serde(default)]
    pub fs_write: AccessSpec,
    #[serde(default)]
    pub network: AccessSpec,
    /// 预留扩展点（未来新增权限项）
    #[serde(default)]
    pub extra: Value,
}

/// **插件配置文件容器**（对应 config.json）
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginConfigInfo {
    /// 权限配置
    #[serde(default)]
    pub access: PluginAccessConfig,
    /// 跨分组公共参数，始终生效
    #[serde(default)]
    pub common: Vec<PluginConfigItem>,
    /// 互斥分组（模式选择），运行态只激活一个；可为空
    #[serde(default)]
    pub groups: Vec<PluginConfigGroup>,
}

impl PluginConfigInfo {
    /// 是否为分组型插件
    pub fn has_groups(&self) -> bool {
        !self.groups.is_empty()
    }

    /// 按分组标识查找
    pub fn find_group(&self, group: &str) -> Option<&PluginConfigGroup> {
        self.groups.iter().find(|g| g.group == group)
    }

    /// 全部合法分组标识（用于 UI 下拉 / 报错提示）
    pub fn group_keys(&self) -> Vec<&str> {
        self.groups.iter().map(|g| g.group.as_str()).collect()
    }

    /// 指定分组激活时的生效参数集 = common + 该分组 params
    pub fn effective_items(&self, group: Option<&str>) -> Vec<&PluginConfigItem> {
        let mut items: Vec<&PluginConfigItem> = self.common.iter().collect();
        if let Some(g) = group.and_then(|g| self.find_group(g)) {
            items.extend(g.params.iter());
        }
        items
    }
}

/// **配置分组**（互斥的模式类型，如 oss / s3 / local）
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginConfigGroup {
    /// 分组标识（运行态 registry_config 的 `group` 值）
    pub group: String,
    /// 分组标题
    pub title: String,
    /// 分组描述
    #[serde(default)]
    pub description: String,
    /// 该分组独有参数
    #[serde(default)]
    pub params: Vec<PluginConfigItem>,
}

/// **单个配置项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginConfigItem {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub config_type: UploadConfigType,
    #[serde(default)]
    pub default_value: Value,
    /// 通用约束：是否必填（缺失 / null / 空串 / 空数组 均视为未填）
    #[serde(default)]
    pub required: bool,
    pub form: PluginFormSpec,
}

/// **表单控件描述 + 控件级约束**（internally tagged，tag = "type"）
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginFormSpec {
    /// 文本输入；secret = true 为密码框
    Text {
        #[serde(default)]
        secret: bool,
        #[serde(default)]
        min_len: Option<usize>,
        #[serde(default)]
        max_len: Option<usize>,
        /// 正则（Rust regex 语法，按 is_match 语义判定）
        #[serde(default)]
        pattern: Option<String>,
        /// 输入占位提示（纯 UI）
        #[serde(default)]
        placeholder: Option<String>,
    },
    /// 开关（bool）
    Switch {},
    /// 选择框
    Select {
        #[serde(default)]
        options: Vec<PluginValueOption>,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        allow_custom: bool,
        /// multiple = true 时的选中数量约束
        #[serde(default)]
        min_items: Option<usize>,
        #[serde(default)]
        max_items: Option<usize>,
    },
    /// 数字输入
    Number {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        /// 步进；提供时校验 (v - min.unwrap_or(0)) 是否为 step 的整倍数
        #[serde(default)]
        step: Option<f64>,
        /// 是否只允许整数
        #[serde(default)]
        integer: bool,
    },
}

/// **候选项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginValueOption {
    pub label: String,
    pub value: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_info_common_groups_deserialize() {
        let json = r#"{
            "access": { "fs_read": true },
            "common": [
                {
                    "key": "retry_times",
                    "title": "重试次数",
                    "config_type": "Default",
                    "default_value": 3,
                    "form": { "type": "number", "min": 0, "max": 10, "integer": true }
                }
            ],
            "groups": [
                {
                    "group": "oss",
                    "title": "阿里云 OSS",
                    "params": [
                        {
                            "key": "endpoint",
                            "title": "Endpoint",
                            "config_type": "Default",
                            "default_value": "",
                            "required": true,
                            "form": { "type": "text", "pattern": "^https?://.+" }
                        }
                    ]
                },
                { "group": "local", "title": "本地存储", "params": [] }
            ]
        }"#;
        let info: PluginConfigInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.common.len(), 1);
        assert!(info.has_groups());
        assert_eq!(info.group_keys(), vec!["oss", "local"]);
        assert!(info.find_group("oss").is_some());
        assert!(info.find_group("s3").is_none());

        let items = info.effective_items(Some("oss"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "retry_times");
        assert_eq!(items[1].key, "endpoint");
        assert!(items[1].required);

        let items_none = info.effective_items(None);
        assert_eq!(items_none.len(), 1);

        let items_unknown = info.effective_items(Some("s3"));
        assert_eq!(items_unknown.len(), 1);
    }

    #[test]
    fn test_form_spec_switch_number_deserialize() {
        let sw: PluginFormSpec = serde_json::from_str(r#"{ "type": "switch" }"#).unwrap();
        assert!(matches!(sw, PluginFormSpec::Switch {}));

        let num: PluginFormSpec =
            serde_json::from_str(r#"{ "type": "number", "min": 1, "max": 8, "step": 0.5 }"#)
                .unwrap();
        assert!(matches!(
            num,
            PluginFormSpec::Number {
                min: Some(1.0),
                max: Some(8.0),
                step: Some(0.5),
                integer: false
            }
        ));
    }

    #[test]
    fn test_form_spec_select_text_deserialize() {
        let json = r#"{
            "type": "select",
            "options": [{"label":"图片","value":"image"}],
            "multiple": true,
            "allow_custom": true,
            "max_items": 5
        }"#;
        let form: PluginFormSpec = serde_json::from_str(json).unwrap();
        assert!(matches!(
            form,
            PluginFormSpec::Select {
                multiple: true,
                allow_custom: true,
                max_items: Some(5),
                ..
            }
        ));

        let txt: PluginFormSpec =
            serde_json::from_str(r#"{ "type": "text", "secret": true, "max_len": 32 }"#).unwrap();
        assert!(matches!(
            txt,
            PluginFormSpec::Text {
                secret: true,
                max_len: Some(32),
                ..
            }
        ));
    }

    #[test]
    fn test_form_spec_serde_default_omitted() {
        let form: PluginFormSpec = serde_json::from_str(r#"{ "type": "text" }"#).unwrap();
        assert!(matches!(
            form,
            PluginFormSpec::Text {
                secret: false,
                min_len: None,
                max_len: None,
                pattern: None,
                placeholder: None
            }
        ));

        let form2: PluginFormSpec = serde_json::from_str(r#"{ "type": "select" }"#).unwrap();
        assert!(matches!(
            form2,
            PluginFormSpec::Select {
                options,
                multiple: false,
                allow_custom: false,
                min_items: None,
                max_items: None
            } if options.is_empty()
        ));
    }

    #[test]
    fn test_config_info_default_empty() {
        let d = PluginConfigInfo::default();
        assert!(d.common.is_empty());
        assert!(d.groups.is_empty());
        assert!(!d.has_groups());
        assert!(matches!(d.access.fs_read, AccessSpec::Flag(false)));
        assert!(matches!(d.access.fs_write, AccessSpec::Flag(false)));
        assert!(matches!(d.access.network, AccessSpec::Flag(false)));
    }

    #[test]
    fn test_plugin_access_config_deserialize() {
        let json = r#"{
            "fs_read": ["/tmp/a", "/data/b"],
            "fs_write": true,
            "network": false
        }"#;
        let acc: PluginAccessConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(acc.fs_read, AccessSpec::Allowlist(p) if p.len() == 2));
        assert!(matches!(acc.fs_write, AccessSpec::Flag(true)));
        assert!(matches!(acc.network, AccessSpec::Flag(false)));
    }

    #[test]
    fn test_config_item_required_default_false() {
        let json = r#"{
            "key": "k", "title": "t", "config_type": "Default",
            "form": { "type": "switch" }
        }"#;
        let item: PluginConfigItem = serde_json::from_str(json).unwrap();
        assert!(!item.required);
        assert_eq!(item.description, "");
        assert!(item.default_value.is_null());
    }
}
