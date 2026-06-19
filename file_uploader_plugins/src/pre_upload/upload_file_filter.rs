use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;
use glob::Pattern;
use std::sync::Arc;
use tracing::{info, warn};

pub struct UploadFileFilter;

/// 解析四项配置：pass_type / reject_type / pass_name（glob 字符串）+ max_size（字节）。
fn parse_config(
    config_info: &Arc<Option<serde_json::Value>>,
) -> (Vec<String>, Vec<String>, Vec<String>, Option<u64>) {
    (
        config_util::get_list(config_info, "pass_type"),
        config_util::get_list(config_info, "reject_type"),
        config_util::get_list(config_info, "pass_name"),
        config_util::get_size(config_info, "max_size"),
    )
}

/// 编译 glob 模式；无效模式记 warn 并跳过。
fn compile_patterns(items: &[String]) -> Vec<Pattern> {
    items
        .iter()
        .filter_map(|s| match Pattern::new(s) {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("upload_file_filter: invalid glob pattern '{}': {}", s, e);
                None
            }
        })
        .collect()
}

/// 三维度保留判定：
/// (pass_type 空 ∨ 命中) ∧ (未命中 reject_type) ∧ (pass_name 空 ∨ 命中) ∧ (size 不超限)
fn keep(
    f: &UploadFileData,
    pass_type: &[Pattern],
    reject_type: &[Pattern],
    pass_name: &[Pattern],
    max_size: Option<u64>,
) -> bool {
    let type_ok = pass_type.is_empty() || pass_type.iter().any(|p| p.matches(&f.file_type));
    let reject_ok = !reject_type.iter().any(|p| p.matches(&f.file_type));
    let name_ok = pass_name.is_empty() || pass_name.iter().any(|p| p.matches(&f.name));
    let size_ok = match max_size {
        Some(m) if m > 0 => (f.size as u64) <= m,
        _ => true,
    };
    type_ok && reject_ok && name_ok && size_ok
}

impl UploadPlugin for UploadFileFilter {
    fn name(&self) -> &'static str {
        "upload_file_filter"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::PreUpload
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        info!(
            "upload_file_filter input: {}",
            serde_json::to_string(ctx).unwrap_or_else(|_| "input error".to_string())
        );

        let (pass_type_raw, reject_type_raw, pass_name_raw, max_size) = parse_config(&ctx.config_info);
        let pass_type = compile_patterns(&pass_type_raw);
        let reject_type = compile_patterns(&reject_type_raw);
        let pass_name = compile_patterns(&pass_name_raw);

        let total = ctx.file_list.len();
        let filtered: Vec<_> = ctx
            .file_list
            .iter()
            .filter(|f| keep(f, &pass_type, &reject_type, &pass_name, max_size))
            .cloned()
            .collect();
        let passed = filtered.len();

        if filtered.is_empty() {
            return UploadOutputCtx::failed(format!(
                "upload_file_filter: all {} file(s) rejected (pass_type={:?}, reject_type={:?}, pass_name={:?}, max_size={:?})",
                total, pass_type_raw, reject_type_raw, pass_name_raw, max_size
            ));
        }

        UploadOutputCtx::success_files(
            format!("upload_file_filter: {}/{} passed", passed, total),
            filtered,
        )
    }

    fn on_load(&self) {
        info!("upload_file_filter: loading");
    }

    fn on_unload(&self) {
        info!("upload_file_filter: unloading");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::enums::FileDataType;
    use serde_json::Value;

    fn file(name: &str, file_type: &str, size: usize) -> Arc<UploadFileData> {
        Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            format!("/tmp/{name}"),
            name.to_string(),
            name.to_string(),
            file_type.to_string(),
            size,
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
        UploadFileFilter.execute(&ctx)
    }

    #[test]
    fn pass_type_keeps_only_matching() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn reject_type_drops_matching() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"reject_type": ["text/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn pass_name_glob_matches_suffix() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_name": ["*.png"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn pass_name_empty_passes_all() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_name": []});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn max_size_filters_oversized() {
        let files = vec![file("small", "image/png", 100), file("big", "image/png", 2_000_000)];
        let cfg = serde_json::json!({"max_size": "1mb"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "small");
    }

    #[test]
    fn max_size_zero_is_unlimited() {
        let files = vec![file("big", "image/png", 99_999_999)];
        let cfg = serde_json::json!({"max_size": "0"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn max_size_numeric_value_works() {
        let files = vec![file("a", "image/png", 50), file("b", "image/png", 200)];
        let cfg = serde_json::json!({"max_size": 100});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a");
    }

    #[test]
    fn combine_all_three_dims() {
        let files = vec![
            file("ok.png", "image/png", 100),
            file("ok.txt", "text/plain", 100),
            file("big.png", "image/png", 9_000_000),
        ];
        let cfg = serde_json::json!({"pass_type": ["image/*"], "pass_name": ["*.png"], "max_size": "1mb"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "ok.png");
    }

    #[test]
    fn empty_config_passes_all() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let out = run(files, None);
        assert_eq!(out.file_list.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn all_rejected_returns_failed() {
        let files = vec![file("a.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
        assert!(out.file_list.is_none());
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        let files = vec![file("a.png", "image/png", 1)];
        let cfg = serde_json::json!({"pass_type": ["["]});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let plugin = UploadFileFilter;
        plugin.on_load();
        plugin.on_unload();
    }
}
