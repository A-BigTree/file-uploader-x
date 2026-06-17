use serde_json::Value;
use std::sync::Arc;

/// 从 config_info（Arc<Option<Value>>）读取某 key 的字符串值。
pub fn get_str(config: &Arc<Option<Value>>, key: &str) -> Option<String> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 读取布尔值。
pub fn get_bool(config: &Arc<Option<Value>>, key: &str) -> Option<bool> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
}

/// 读取字符串数组（非数组 / 元素非字符串均跳过）。缺失或 None → 空 Vec。
pub fn get_list(config: &Arc<Option<Value>>, key: &str) -> Vec<String> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: &str) -> Arc<Option<Value>> {
        Arc::new(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn get_str_present() {
        let c = cfg(r#"{"k":"v"}"#);
        assert_eq!(get_str(&c, "k"), Some("v".to_string()));
    }

    #[test]
    fn get_str_missing_or_none() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert_eq!(get_str(&none_cfg, "k"), None);
        let c = cfg(r#"{"other":1}"#);
        assert_eq!(get_str(&c, "k"), None);
    }

    #[test]
    fn get_str_wrong_type() {
        let c = cfg(r#"{"k":123}"#);
        assert_eq!(get_str(&c, "k"), None);
    }

    #[test]
    fn get_bool_present() {
        let c = cfg(r#"{"flag":true}"#);
        assert_eq!(get_bool(&c, "flag"), Some(true));
    }

    #[test]
    fn get_bool_wrong_type() {
        let c = cfg(r#"{"flag":"yes"}"#);
        assert_eq!(get_bool(&c, "flag"), None);
    }

    #[test]
    fn get_list_present() {
        let c = cfg(r#"{"items":["a","b"]}"#);
        assert_eq!(
            get_list(&c, "items"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn get_list_missing_returns_empty() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert!(get_list(&none_cfg, "items").is_empty());
        let c = cfg(r#"{"other":1}"#);
        assert!(get_list(&c, "items").is_empty());
    }

    #[test]
    fn get_list_non_string_elements_skipped() {
        let c = cfg(r#"{"items":["a", 1, "b"]}"#);
        assert_eq!(
            get_list(&c, "items"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn get_list_not_array_returns_empty() {
        let c = cfg(r#"{"items":"x"}"#);
        assert!(get_list(&c, "items").is_empty());
    }
}
