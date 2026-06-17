use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use glob::Pattern;
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, warn};

pub struct FileTypeFilter;

/// 从 config_info 读出某 key 对应的字符串列表（缺失/非数组 → 空）。
fn parse_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// 解析 pass_type / reject_type。config_info 为 None → 双空（全部允许）。
fn parse_config(config_info: &Arc<Option<Value>>) -> (Vec<String>, Vec<String>) {
    match config_info.as_ref() {
        Some(v) => (parse_list(v, "pass_type"), parse_list(v, "reject_type")),
        None => (vec![], vec![]),
    }
}

/// 编译 glob 模式；无效模式记 warn 并跳过。
fn compile_patterns(items: &[String]) -> Vec<Pattern> {
    items
        .iter()
        .filter_map(|s| match Pattern::new(s) {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("file_type_filter: invalid glob pattern '{}': {}", s, e);
                None
            }
        })
        .collect()
}

/// 保留条件：(pass 为空 ∨ 命中任一 pass) ∧ (未命中任一 reject)
fn keep(file_type: &str, pass: &[Pattern], reject: &[Pattern]) -> bool {
    let pass_ok = pass.is_empty() || pass.iter().any(|p| p.matches(file_type));
    let reject_ok = !reject.iter().any(|p| p.matches(file_type));
    pass_ok && reject_ok
}

impl UploadPlugin for FileTypeFilter {
    fn name(&self) -> &'static str {
        "file-type-filter"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::PreUpload
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        info!(
            "file-type-filter input: {}",
            serde_json::to_string(ctx).unwrap_or_else(|_| "input error".to_string())
        );

        let (pass_raw, reject_raw) = parse_config(&ctx.config_info);
        let pass = compile_patterns(&pass_raw);
        let reject = compile_patterns(&reject_raw);

        let total = ctx.file_list.len();
        let filtered: Vec<_> = ctx
            .file_list
            .iter()
            .filter(|f| keep(&f.file_type, &pass, &reject))
            .cloned()
            .collect();
        let passed = filtered.len();

        if filtered.is_empty() {
            return UploadOutputCtx {
                result: OutputResultType::Failed,
                message: format!(
                    "file_type_filter: all {} file(s) rejected (pass={:?}, reject={:?})",
                    total, pass_raw, reject_raw
                ),
                file_list: None,
                extra_info: None,
            };
        }

        UploadOutputCtx {
            result: OutputResultType::Success,
            message: format!("file_type_filter: {}/{} passed", passed, total),
            file_list: Some(filtered),
            extra_info: None,
        }
    }

    fn on_load(&self) {
        info!("file_type_filter: loading");
    }

    fn on_unload(&self) {
        info!("file_type_filter: unloading");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::ctx::UploadFileData;
    use file_uploader_sdk::models::enums::FileDataType;

    fn file(name: &str, file_type: &str) -> Arc<UploadFileData> {
        Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            format!("/tmp/{name}"),
            name.to_string(),
            name.to_string(),
            file_type.to_string(),
            0,
        ))
    }

    fn run(files: Vec<Arc<UploadFileData>>, config: Option<Value>) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file_list: files,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir: None,
        };
        FileTypeFilter.execute(&ctx)
    }

    #[test]
    fn pass_filter_keeps_only_matching() {
        let files = vec![file("a.png", "image/png"), file("b.txt", "text/plain")];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn reject_filter_drops_matching() {
        let files = vec![file("a.png", "image/png"), file("b.txt", "text/plain")];
        let cfg = serde_json::json!({"reject_type": ["text/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn empty_config_passes_all() {
        let files = vec![file("a.png", "image/png"), file("b.txt", "text/plain")];
        let out = run(files, None);
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn combine_pass_and_reject() {
        let files = vec![
            file("a.png", "image/png"),
            file("b.jpg", "image/jpeg"),
            file("c.txt", "text/plain"),
        ];
        let cfg = serde_json::json!({"pass_type": ["image/*"], "reject_type": ["image/jpeg"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn all_rejected_returns_failed() {
        let files = vec![file("a.txt", "text/plain")];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
        assert!(out.file_list.is_none());
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        let files = vec![file("a.png", "image/png")];
        // "[" 是无效 glob → 跳过 → pass 为空 → 全部通过
        let cfg = serde_json::json!({"pass_type": ["["]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let plugin = FileTypeFilter;
        plugin.on_load();
        plugin.on_unload();
    }
}
