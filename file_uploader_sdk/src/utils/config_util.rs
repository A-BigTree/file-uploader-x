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

/// 解析带单位的大小字符串为字节数（二进制 1024 进制）。
/// 支持：纯数字（按字节）或 数字+单位；单位大小写不敏感。
///   b/byte/bytes → 1；k/kb → 1024；m/mb → 1024²；g/gb → 1024³；t/tb → 1024⁴
/// 非法输入 → None。
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let split = s.find(|c: char| c.is_ascii_alphabetic());
    let (num_part, unit_part) = match split {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let num: u64 = num_part.parse().ok()?;
    let mult: u64 = match unit_part.to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024 * 1024,
        "g" | "gb" => 1024u64 * 1024 * 1024,
        "t" | "tb" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    num.checked_mul(mult)
}

/// 读取某 key 的大小：字符串走 `parse_size`，数字走 `as_u64`。缺失/无法解析 → None。
pub fn get_size(config: &Arc<Option<Value>>, key: &str) -> Option<u64> {
    let v = config.as_ref().as_ref()?.get(key)?;
    match v {
        Value::String(s) => parse_size(s),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
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

    #[test]
    fn parse_size_plain_bytes() {
        assert_eq!(super::parse_size("1024"), Some(1024));
    }

    #[test]
    fn parse_size_units_binary() {
        assert_eq!(super::parse_size("1kb"), Some(1024));
        assert_eq!(super::parse_size("1mb"), Some(1024 * 1024));
        assert_eq!(super::parse_size("1gb"), Some(1024u64 * 1024 * 1024));
        assert_eq!(super::parse_size("1g"), Some(1024u64 * 1024 * 1024));
        assert_eq!(super::parse_size("1tb"), Some(1024u64 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_size_case_insensitive_and_trimmed() {
        assert_eq!(super::parse_size("  10MB "), Some(10 * 1024 * 1024));
        assert_eq!(super::parse_size("2Kb"), Some(2 * 1024));
    }

    #[test]
    fn parse_size_invalid() {
        assert_eq!(super::parse_size(""), None);
        assert_eq!(super::parse_size("abc"), None);
        assert_eq!(super::parse_size("1xb"), None);
    }

    #[test]
    fn get_size_from_string() {
        let c = cfg(r#"{"max":"2mb"}"#);
        assert_eq!(super::get_size(&c, "max"), Some(2 * 1024 * 1024));
    }

    #[test]
    fn get_size_from_number() {
        let c = cfg(r#"{"max":1048576}"#);
        assert_eq!(super::get_size(&c, "max"), Some(1048576));
    }

    #[test]
    fn get_size_missing_or_invalid() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert_eq!(super::get_size(&none_cfg, "max"), None);
        let c = cfg(r#"{"max":"abc"}"#);
        assert_eq!(super::get_size(&c, "max"), None);
    }
}
