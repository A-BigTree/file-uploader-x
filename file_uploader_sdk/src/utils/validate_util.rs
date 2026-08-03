use crate::models::config_schema::{PluginConfigInfo, PluginConfigItem, PluginFormSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 运行态配置中标识激活分组的保留字段名
pub const GROUP_KEY: &str = "group";

/// 浮点比较容差（用于 step 整倍数判定）
const EPSILON: f64 = 1e-9;

/// **校验失败原因**
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidationReason {
    /// 分组型插件未提供 group
    GroupMissing { expected: Vec<String> },
    /// group 值不在声明的分组内
    GroupUnknown {
        found: String,
        expected: Vec<String>,
    },
    /// 非分组型插件却传入了 group
    GroupNotAllowed,
    /// 必填项未填
    Required,
    /// 值的 JSON 类型与控件不匹配
    TypeMismatch {
        expected: String,
        found: String,
    },
    TooShort {
        min: usize,
        actual: usize,
    },
    TooLong {
        max: usize,
        actual: usize,
    },
    PatternMismatch {
        pattern: String,
    },
    /// schema 自身声明的正则非法
    InvalidPattern {
        pattern: String,
        error: String,
    },
    NotInOptions {
        allowed: Vec<Value>,
    },
    TooFewItems {
        min: usize,
        actual: usize,
    },
    TooManyItems {
        max: usize,
        actual: usize,
    },
    OutOfRange {
        min: Option<f64>,
        max: Option<f64>,
    },
    NotInteger,
    StepMismatch {
        step: f64,
    },
    /// 顶层配置不是 JSON object
    NotAnObject,
    /// 插件 validate_params 拒绝
    PluginRejected {
        message: String,
    },
    /// 严格模式下的未声明字段
    UnknownKey,
}

impl ValidationReason {
    /// 构造类型不匹配原因
    pub fn mismatch(expected: &str, found: &Value) -> Self {
        Self::TypeMismatch {
            expected: expected.to_string(),
            found: type_name(found).to_string(),
        }
    }
}

impl std::fmt::Display for ValidationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GroupMissing { expected } => {
                write!(f, "缺少分组标识 group，可选值: {:?}", expected)
            }
            Self::GroupUnknown { found, expected } => {
                write!(f, "未知分组 '{}'，可选值: {:?}", found, expected)
            }
            Self::GroupNotAllowed => write!(f, "该插件未声明分组，不应传入 group"),
            Self::Required => write!(f, "必填项未填"),
            Self::TypeMismatch { expected, found } => {
                write!(f, "类型不匹配：期望 {}，实际 {}", expected, found)
            }
            Self::TooShort { min, actual } => write!(f, "长度过短：最小 {}，实际 {}", min, actual),
            Self::TooLong { max, actual } => write!(f, "长度过长：最大 {}，实际 {}", max, actual),
            Self::PatternMismatch { pattern } => write!(f, "不满足正则 {}", pattern),
            Self::InvalidPattern { pattern, error } => {
                write!(f, "schema 正则非法 '{}': {}", pattern, error)
            }
            Self::NotInOptions { allowed } => write!(f, "值不在候选项内，候选: {:?}", allowed),
            Self::TooFewItems { min, actual } => {
                write!(f, "选中过少：最少 {}，实际 {}", min, actual)
            }
            Self::TooManyItems { max, actual } => {
                write!(f, "选中过多：最多 {}，实际 {}", max, actual)
            }
            Self::OutOfRange { min, max } => write!(f, "数值越界：min={:?}, max={:?}", min, max),
            Self::NotInteger => write!(f, "必须为整数"),
            Self::StepMismatch { step } => write!(f, "必须为步进 {} 的整倍数", step),
            Self::NotAnObject => write!(f, "配置必须是 JSON 对象"),
            Self::PluginRejected { message } => write!(f, "插件校验拒绝: {}", message),
            Self::UnknownKey => write!(f, "未声明的配置项"),
        }
    }
}

/// **单条校验错误**
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// 出错的参数 key；分组级 / 顶层错误为 None
    pub key: Option<String>,
    /// 该 key 所属分组；common 参数为 None
    pub group: Option<String>,
    /// 参数标题（便于前端直接展示）
    pub title: Option<String>,
    pub reason: ValidationReason,
}

impl ValidationError {
    /// 顶层 / 分组级错误
    pub fn top(reason: ValidationReason) -> Self {
        Self {
            key: None,
            group: None,
            title: None,
            reason,
        }
    }

    /// 参数级错误
    pub fn of(item: &PluginConfigItem, group: Option<&str>, reason: ValidationReason) -> Self {
        Self {
            key: Some(item.key.clone()),
            group: group.map(|g| g.to_string()),
            title: Some(item.title.clone()),
            reason,
        }
    }

    /// 仅有 key 的错误（如 UnknownKey）
    pub fn of_key(key: &str, reason: ValidationReason) -> Self {
        Self {
            key: Some(key.to_string()),
            group: None,
            title: None,
            reason,
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.group, &self.key) {
            (Some(g), Some(k)) => write!(f, "[{}.{}] {}", g, k, self.reason),
            (None, Some(k)) => write!(f, "[{}] {}", k, self.reason),
            _ => write!(f, "{}", self.reason),
        }
    }
}

/// 把多条错误折叠为单行字符串（用于 `UploadError` 载荷）
pub fn errors_to_string(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// **校验选项**
#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    /// 是否拒绝 schema 未声明的多余 key
    pub strict_unknown_keys: bool,
}

/// 判空语义：缺失 / null / 空串 / 空数组 → true
pub fn is_empty_value(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        _ => false,
    }
}

/// JSON 值的类型名（用于错误信息）
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 是否为标量（select 单选 / 多选元素允许的类型）
fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::Bool(_) | Value::Number(_) | Value::String(_))
}

/// **对外主入口**：校验运行态扁平配置（含保留字段 `group`）
pub fn validate_plugin_config(
    config: &PluginConfigInfo,
    values: &Value,
) -> Result<(), Vec<ValidationError>> {
    validate_plugin_config_with(config, values, &ValidateOptions::default())
}

/// 兼容 `registry_config: Option<Value>`（None 等价空对象）
pub fn validate_plugin_config_opt(
    config: &PluginConfigInfo,
    values: &Option<Value>,
) -> Result<(), Vec<ValidationError>> {
    match values {
        Some(v) => validate_plugin_config(config, v),
        None => validate_plugin_config(config, &Value::Object(serde_json::Map::new())),
    }
}

/// 带选项的完整校验（错误全部累积，不短路）
pub fn validate_plugin_config_with(
    config: &PluginConfigInfo,
    values: &Value,
    opts: &ValidateOptions,
) -> Result<(), Vec<ValidationError>> {
    let Some(map) = values.as_object() else {
        return Err(vec![ValidationError::top(ValidationReason::NotAnObject)]);
    };

    let mut errors: Vec<ValidationError> = Vec::new();

    // ---- ① 分组校验 ----
    let raw_group = map.get(GROUP_KEY);
    let mut active_group: Option<String> = None;

    if config.has_groups() {
        let expected: Vec<String> = config
            .group_keys()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        match raw_group.and_then(|v| v.as_str()) {
            None => errors.push(ValidationError::top(ValidationReason::GroupMissing {
                expected,
            })),
            Some(g) if config.find_group(g).is_none() => {
                errors.push(ValidationError::top(ValidationReason::GroupUnknown {
                    found: g.to_string(),
                    expected,
                }))
            }
            Some(g) => active_group = Some(g.to_string()),
        }
    } else if raw_group.is_some_and(|v| !v.is_null()) {
        errors.push(ValidationError::top(ValidationReason::GroupNotAllowed));
    }

    // ---- ② 逐项校验生效参数 ----
    let common_keys: std::collections::HashSet<&str> =
        config.common.iter().map(|i| i.key.as_str()).collect();

    for item in config.effective_items(active_group.as_deref()) {
        // common 参数的 group 记为 None，分组参数记为激活分组
        let item_group = if common_keys.contains(item.key.as_str()) {
            None
        } else {
            active_group.as_deref()
        };
        errors.extend(validate_item(item, item_group, map.get(&item.key)));
    }

    // ---- ③ 严格模式：未声明字段 ----
    if opts.strict_unknown_keys {
        let declared: std::collections::HashSet<&str> = config
            .effective_items(active_group.as_deref())
            .into_iter()
            .map(|i| i.key.as_str())
            .collect();
        for key in map.keys() {
            if key != GROUP_KEY && !declared.contains(key.as_str()) {
                errors.push(ValidationError::of_key(key, ValidationReason::UnknownKey));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// **单项校验**（前端逐字段实时校验可复用）
pub fn validate_item(
    item: &PluginConfigItem,
    group: Option<&str>,
    value: Option<&Value>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // required 判空；空值（无论必填与否）不再跑后续约束
    if is_empty_value(value) {
        if item.required {
            errors.push(ValidationError::of(item, group, ValidationReason::Required));
        }
        return errors;
    }

    let v = value.expect("non-empty value must exist");

    match &item.form {
        PluginFormSpec::Text {
            min_len,
            max_len,
            pattern,
            ..
        } => {
            let Some(s) = v.as_str() else {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::mismatch("string", v),
                ));
                return errors;
            };
            let len = s.chars().count();
            if let Some(min) = min_len
                && len < *min
            {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::TooShort {
                        min: *min,
                        actual: len,
                    },
                ));
            }
            if let Some(max) = max_len
                && len > *max
            {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::TooLong {
                        max: *max,
                        actual: len,
                    },
                ));
            }
            if let Some(p) = pattern {
                match regex::Regex::new(p) {
                    Ok(re) => {
                        if !re.is_match(s) {
                            errors.push(ValidationError::of(
                                item,
                                group,
                                ValidationReason::PatternMismatch { pattern: p.clone() },
                            ));
                        }
                    }
                    Err(e) => errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::InvalidPattern {
                            pattern: p.clone(),
                            error: e.to_string(),
                        },
                    )),
                }
            }
        }

        PluginFormSpec::Switch {} => {
            if !v.is_boolean() {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::mismatch("bool", v),
                ));
            }
        }

        PluginFormSpec::Number {
            min,
            max,
            step,
            integer,
        } => {
            let Some(n) = v.as_f64() else {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::mismatch("number", v),
                ));
                return errors;
            };
            let out_of_range =
                min.is_some_and(|m| n < m - EPSILON) || max.is_some_and(|m| n > m + EPSILON);
            if out_of_range {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::OutOfRange {
                        min: *min,
                        max: *max,
                    },
                ));
            }
            if *integer && (n.fract().abs() > EPSILON) {
                errors.push(ValidationError::of(
                    item,
                    group,
                    ValidationReason::NotInteger,
                ));
            }
            if let Some(st) = step
                && *st > EPSILON
            {
                let base = min.unwrap_or(0.0);
                let quotient = (n - base) / st;
                if (quotient - quotient.round()).abs() > 1e-6 {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::StepMismatch { step: *st },
                    ));
                }
            }
        }

        PluginFormSpec::Select {
            options,
            multiple,
            allow_custom,
            min_items,
            max_items,
        } => {
            let allowed: Vec<Value> = options.iter().map(|o| o.value.clone()).collect();

            if *multiple {
                let Some(arr) = v.as_array() else {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::mismatch("array", v),
                    ));
                    return errors;
                };
                for elem in arr {
                    if !is_scalar(elem) {
                        errors.push(ValidationError::of(
                            item,
                            group,
                            ValidationReason::mismatch("scalar", elem),
                        ));
                    }
                }
                if !*allow_custom {
                    for elem in arr {
                        if !allowed.contains(elem) {
                            errors.push(ValidationError::of(
                                item,
                                group,
                                ValidationReason::NotInOptions {
                                    allowed: allowed.clone(),
                                },
                            ));
                            break;
                        }
                    }
                }
                if let Some(min) = min_items
                    && arr.len() < *min
                {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::TooFewItems {
                            min: *min,
                            actual: arr.len(),
                        },
                    ));
                }
                if let Some(max) = max_items
                    && arr.len() > *max
                {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::TooManyItems {
                            max: *max,
                            actual: arr.len(),
                        },
                    ));
                }
            } else {
                if !is_scalar(v) {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::mismatch("scalar", v),
                    ));
                    return errors;
                }
                if !*allow_custom && !allowed.contains(v) {
                    errors.push(ValidationError::of(
                        item,
                        group,
                        ValidationReason::NotInOptions { allowed },
                    ));
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::config_schema::{PluginConfigGroup, PluginValueOption};
    use crate::models::enums::UploadConfigType;
    use serde_json::json;

    fn item(key: &str, required: bool, form: PluginFormSpec) -> PluginConfigItem {
        PluginConfigItem {
            key: key.to_string(),
            title: format!("标题-{key}"),
            description: String::new(),
            config_type: UploadConfigType::Default,
            default_value: Value::Null,
            required,
            form,
        }
    }

    fn text(
        secret: bool,
        min_len: Option<usize>,
        max_len: Option<usize>,
        pattern: Option<&str>,
    ) -> PluginFormSpec {
        PluginFormSpec::Text {
            secret,
            min_len,
            max_len,
            pattern: pattern.map(|s| s.to_string()),
            placeholder: None,
        }
    }

    fn select(
        values: &[&str],
        multiple: bool,
        allow_custom: bool,
        min_items: Option<usize>,
        max_items: Option<usize>,
    ) -> PluginFormSpec {
        PluginFormSpec::Select {
            options: values
                .iter()
                .map(|v| PluginValueOption {
                    label: v.to_string(),
                    value: json!(v),
                })
                .collect(),
            multiple,
            allow_custom,
            min_items,
            max_items,
        }
    }

    fn cfg(common: Vec<PluginConfigItem>, groups: Vec<PluginConfigGroup>) -> PluginConfigInfo {
        PluginConfigInfo {
            access: Default::default(),
            common,
            groups,
        }
    }

    fn reasons(e: &[ValidationError]) -> Vec<ValidationReason> {
        e.iter().map(|x| x.reason.clone()).collect()
    }

    // ---- ① 顶层结构 ----
    #[test]
    fn test_not_an_object() {
        let c = cfg(vec![], vec![]);
        let err = validate_plugin_config(&c, &json!([1, 2])).unwrap_err();
        assert_eq!(reasons(&err), vec![ValidationReason::NotAnObject]);
    }

    // ---- ②③④ 分组 ----
    #[test]
    fn test_group_missing() {
        let c = cfg(
            vec![],
            vec![PluginConfigGroup {
                group: "oss".into(),
                title: "OSS".into(),
                description: String::new(),
                params: vec![],
            }],
        );
        let err = validate_plugin_config(&c, &json!({})).unwrap_err();
        assert_eq!(
            reasons(&err),
            vec![ValidationReason::GroupMissing {
                expected: vec!["oss".to_string()]
            }]
        );
    }

    #[test]
    fn test_group_unknown() {
        let c = cfg(
            vec![],
            vec![PluginConfigGroup {
                group: "oss".into(),
                title: "OSS".into(),
                description: String::new(),
                params: vec![],
            }],
        );
        let err = validate_plugin_config(&c, &json!({"group": "s3"})).unwrap_err();
        assert_eq!(
            reasons(&err),
            vec![ValidationReason::GroupUnknown {
                found: "s3".into(),
                expected: vec!["oss".to_string()]
            }]
        );
    }

    #[test]
    fn test_group_not_allowed() {
        let c = cfg(vec![], vec![]);
        let err = validate_plugin_config(&c, &json!({"group": "oss"})).unwrap_err();
        assert_eq!(reasons(&err), vec![ValidationReason::GroupNotAllowed]);
    }

    #[test]
    fn test_group_ok() {
        let c = cfg(
            vec![],
            vec![PluginConfigGroup {
                group: "oss".into(),
                title: "OSS".into(),
                description: String::new(),
                params: vec![],
            }],
        );
        assert!(validate_plugin_config(&c, &json!({"group": "oss"})).is_ok());
    }

    // ---- ⑤ required 判空四态 ----
    #[test]
    fn test_required_empty_forms() {
        let c = cfg(vec![item("k", true, text(false, None, None, None))], vec![]);
        for v in [json!({}), json!({"k": null}), json!({"k": ""})] {
            let err = validate_plugin_config(&c, &v).unwrap_err();
            assert_eq!(reasons(&err), vec![ValidationReason::Required], "{v}");
        }
        let c2 = cfg(
            vec![item("k", true, select(&["a"], true, true, None, None))],
            vec![],
        );
        let err = validate_plugin_config(&c2, &json!({"k": []})).unwrap_err();
        assert_eq!(reasons(&err), vec![ValidationReason::Required]);
    }

    // ---- ⑥ 非必填空值跳过后续约束 ----
    #[test]
    fn test_optional_empty_skips_constraints() {
        let c = cfg(
            vec![item("k", false, text(false, Some(5), None, Some("^a")))],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": ""})).is_ok());
        assert!(validate_plugin_config(&c, &json!({})).is_ok());
    }

    // ---- ⑦⑧⑨ text ----
    #[test]
    fn test_text_type_mismatch() {
        let c = cfg(vec![item("k", false, text(false, None, None, None))], vec![]);
        let err = validate_plugin_config(&c, &json!({"k": 123})).unwrap_err();
        assert_eq!(
            reasons(&err),
            vec![ValidationReason::TypeMismatch { expected: "string".into(), found: "number".into() }]
        );
    }

    #[test]
    fn test_text_len_bounds() {
        let c = cfg(
            vec![item("k", false, text(false, Some(3), Some(5), None))],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": "abc"})).is_ok());
        assert!(validate_plugin_config(&c, &json!({"k": "abcde"})).is_ok());
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "ab"})).unwrap_err()),
            vec![ValidationReason::TooShort { min: 3, actual: 2 }]
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "abcdef"})).unwrap_err()),
            vec![ValidationReason::TooLong { max: 5, actual: 6 }]
        );
    }

    #[test]
    fn test_text_pattern() {
        let c = cfg(
            vec![item("k", false, text(false, None, None, Some("^https?://")))],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": "https://a.com"})).is_ok());
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "ftp://a"})).unwrap_err()),
            vec![ValidationReason::PatternMismatch {
                pattern: "^https?://".into()
            }]
        );
    }

    #[test]
    fn test_text_invalid_schema_pattern() {
        let c = cfg(
            vec![item("k", false, text(false, None, None, Some("[unclosed")))],
            vec![],
        );
        let err = validate_plugin_config(&c, &json!({"k": "x"})).unwrap_err();
        assert!(matches!(
            err[0].reason,
            ValidationReason::InvalidPattern { .. }
        ));
    }

    // ---- ⑩ switch ----
    #[test]
    fn test_switch_type() {
        let c = cfg(vec![item("k", false, PluginFormSpec::Switch {})], vec![]);
        assert!(validate_plugin_config(&c, &json!({"k": true})).is_ok());
        assert!(validate_plugin_config(&c, &json!({"k": false})).is_ok());
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "true"})).unwrap_err()),
            vec![ValidationReason::TypeMismatch { expected: "bool".into(), found: "string".into() }]
        );
    }

    // ---- ⑪ number ----
    #[test]
    fn test_number_constraints() {
        let form = PluginFormSpec::Number {
            min: Some(1.0),
            max: Some(10.0),
            step: None,
            integer: true,
        };
        let c = cfg(vec![item("k", false, form)], vec![]);
        assert!(validate_plugin_config(&c, &json!({"k": 5})).is_ok());
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "5"})).unwrap_err()),
            vec![ValidationReason::TypeMismatch { expected: "number".into(), found: "string".into() }]
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": 0})).unwrap_err()),
            vec![ValidationReason::OutOfRange {
                min: Some(1.0),
                max: Some(10.0)
            }]
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": 11})).unwrap_err()),
            vec![ValidationReason::OutOfRange {
                min: Some(1.0),
                max: Some(10.0)
            }]
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": 1.5})).unwrap_err()),
            vec![ValidationReason::NotInteger]
        );
    }

    #[test]
    fn test_number_step() {
        let form = PluginFormSpec::Number {
            min: Some(0.0),
            max: Some(10.0),
            step: Some(0.5),
            integer: false,
        };
        let c = cfg(vec![item("k", false, form)], vec![]);
        assert!(validate_plugin_config(&c, &json!({"k": 2.5})).is_ok());
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": 2.3})).unwrap_err()),
            vec![ValidationReason::StepMismatch { step: 0.5 }]
        );
    }

    // ---- ⑫⑬ select ----
    #[test]
    fn test_select_single_not_in_options() {
        let c = cfg(
            vec![item("k", false, select(&["a", "b"], false, false, None, None))],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": "a"})).is_ok());
        assert!(matches!(
            validate_plugin_config(&c, &json!({"k": "z"})).unwrap_err()[0].reason,
            ValidationReason::NotInOptions { .. }
        ));
    }

    #[test]
    fn test_select_multiple() {
        let c = cfg(
            vec![item(
                "k",
                false,
                select(&["a", "b", "c"], true, false, Some(1), Some(2)),
            )],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": ["a"]})).is_ok());
        assert!(validate_plugin_config(&c, &json!({"k": ["a", "b"]})).is_ok());

        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": "a"})).unwrap_err()),
            vec![ValidationReason::TypeMismatch { expected: "array".into(), found: "string".into() }]
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": ["a","b","c"]})).unwrap_err()),
            vec![ValidationReason::TooManyItems { max: 2, actual: 3 }]
        );
        assert!(matches!(
            validate_plugin_config(&c, &json!({"k": ["z"]})).unwrap_err()[0].reason,
            ValidationReason::NotInOptions { .. }
        ));
    }

    #[test]
    fn test_select_multiple_min_items() {
        let c = cfg(
            vec![item(
                "k",
                false,
                select(&["a", "b"], true, true, Some(2), None),
            )],
            vec![],
        );
        assert_eq!(
            reasons(&validate_plugin_config(&c, &json!({"k": ["a"]})).unwrap_err()),
            vec![ValidationReason::TooFewItems { min: 2, actual: 1 }]
        );
    }

    #[test]
    fn test_select_allow_custom_passes_any() {
        let c = cfg(
            vec![item("k", false, select(&["a"], true, true, None, None))],
            vec![],
        );
        assert!(validate_plugin_config(&c, &json!({"k": ["image/*", "zzz"]})).is_ok());
    }

    // ---- ⑭ 多错误累积 ----
    #[test]
    fn test_multiple_errors_accumulated() {
        let c = cfg(
            vec![
                item("a", true, text(false, None, None, None)),
                item("b", false, PluginFormSpec::Switch {}),
            ],
            vec![],
        );
        let err = validate_plugin_config(&c, &json!({"b": "x"})).unwrap_err();
        assert_eq!(err.len(), 2);
        assert_eq!(err[0].key.as_deref(), Some("a"));
        assert_eq!(err[0].title.as_deref(), Some("标题-a"));
        assert_eq!(err[0].group, None);
        assert_eq!(err[1].key.as_deref(), Some("b"));
    }

    // ---- ⑮ 分组参数只在激活分组下校验 ----
    #[test]
    fn test_only_active_group_params_validated() {
        let c = cfg(
            vec![item("t", false, PluginFormSpec::Switch {})],
            vec![
                PluginConfigGroup {
                    group: "oss".into(),
                    title: "OSS".into(),
                    description: String::new(),
                    params: vec![item("endpoint", true, text(false, None, None, None))],
                },
                PluginConfigGroup {
                    group: "local".into(),
                    title: "Local".into(),
                    description: String::new(),
                    params: vec![item("base_dir", true, text(false, None, None, None))],
                },
            ],
        );
        // 激活 local，只需 base_dir，不报 endpoint
        assert!(validate_plugin_config(&c, &json!({"group":"local","base_dir":"/tmp"})).is_ok());

        let err = validate_plugin_config(&c, &json!({"group":"oss"})).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].key.as_deref(), Some("endpoint"));
        assert_eq!(err[0].group.as_deref(), Some("oss"));
        assert_eq!(err[0].reason, ValidationReason::Required);
    }

    // ---- ⑯ strict_unknown_keys ----
    #[test]
    fn test_strict_unknown_keys() {
        let c = cfg(vec![item("k", false, PluginFormSpec::Switch {})], vec![]);
        let values = json!({"k": true, "zzz": 1});
        assert!(validate_plugin_config(&c, &values).is_ok());

        let opts = ValidateOptions {
            strict_unknown_keys: true,
        };
        let err = validate_plugin_config_with(&c, &values, &opts).unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].key.as_deref(), Some("zzz"));
        assert_eq!(err[0].reason, ValidationReason::UnknownKey);
    }

    // ---- ⑰ Option<Value> 入口 ----
    #[test]
    fn test_validate_opt_none() {
        let c_ok = cfg(vec![item("k", false, PluginFormSpec::Switch {})], vec![]);
        assert!(validate_plugin_config_opt(&c_ok, &None).is_ok());

        let c_req = cfg(vec![item("k", true, PluginFormSpec::Switch {})], vec![]);
        assert_eq!(
            reasons(&validate_plugin_config_opt(&c_req, &None).unwrap_err()),
            vec![ValidationReason::Required]
        );
    }

    #[test]
    fn test_errors_to_string() {
        let c = cfg(vec![item("a", true, text(false, None, None, None))], vec![]);
        let err = validate_plugin_config(&c, &json!({})).unwrap_err();
        let s = errors_to_string(&err);
        assert!(s.contains("[a]"), "got: {s}");
        assert!(s.contains("必填"), "got: {s}");
    }

    #[test]
    fn test_is_empty_value() {
        assert!(is_empty_value(None));
        assert!(is_empty_value(Some(&json!(null))));
        assert!(is_empty_value(Some(&json!(""))));
        assert!(is_empty_value(Some(&json!([]))));
        assert!(!is_empty_value(Some(&json!(false))));
        assert!(!is_empty_value(Some(&json!(0))));
        assert!(!is_empty_value(Some(&json!("a"))));
    }
}
