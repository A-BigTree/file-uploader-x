mod render;

use std::collections::HashMap;
use std::sync::Arc;

use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{FileDataType, OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;

use render::{render, OutputFormat, Values};

pub struct CommonOutput;

impl UploadPlugin for CommonOutput {
    fn name(&self) -> &'static str {
        "common_output"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::Output
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        let Some(f) = ctx.file.as_ref() else {
            return UploadOutputCtx::failed("common_output: 未获得可访问 URL");
        };
        if !matches!(f.data_type, FileDataType::NetworkPath) {
            return UploadOutputCtx::failed("common_output: 仅接受网络路径（NetworkPath）文件");
        }
        if !(f.input_path.starts_with("http://") || f.input_path.starts_with("https://")) {
            return UploadOutputCtx::failed("common_output: 未获得可访问 URL");
        }

        let format_str = config_util::get_str(&ctx.config_info, "format")
            .unwrap_or_else(|| "markdown".to_string());
        let format = match OutputFormat::parse(&format_str) {
            Ok(f) => f,
            Err(e) => return UploadOutputCtx::failed(format!("common_output: {e}")),
        };
        let template = config_util::get_str(&ctx.config_info, "template");
        let template_used = template.as_deref().map(str::is_empty).map(|empty| !empty).unwrap_or(false);

        let values = Values {
            name: &f.name,
            url: &f.input_path,
            size: f.size,
            file_type: &f.file_type,
        };
        let output = match render(format, &values, template.as_deref()) {
            Ok(s) => s,
            Err(e) => return UploadOutputCtx::failed(format!("common_output: {e}")),
        };

        let output_format_label = if template_used {
            "template"
        } else {
            format.as_str()
        };

        let mut extra: HashMap<String, String> = HashMap::new();
        extra.insert("output".to_string(), output.clone());
        extra.insert("output_format".to_string(), output_format_label.to_string());
        if let Some(orig) = ctx.extra_info.as_ref() {
            for (k, v) in orig {
                extra.entry(k.clone()).or_insert(v.clone());
            }
        }

        UploadOutputCtx {
            result: OutputResultType::Success,
            message: output,
            file: Some(f.clone()),
            extra_info: Some(extra),
        }
    }

    fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
        if let Some(fmt) = config_util::get_str(&ctx.config_info, "format") {
            OutputFormat::parse(&fmt).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn on_load(&self) {
        tracing::info!("common_output: loading");
    }

    fn on_unload(&self) {
        tracing::info!("common_output: unloading");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::ctx::UploadFileData;
    use serde_json::{json, Value};

    fn file(data_type: FileDataType, url: &str) -> Arc<UploadFileData> {
        let name = url.rsplit_once('/').map(|(_, n)| n).unwrap_or(url).to_string();
        Arc::new(UploadFileData::new(
            data_type,
            url.into(),
            "id1".into(),
            name,
            "image/png".into(),
            12,
        ))
    }

    fn run(f: Option<Arc<UploadFileData>>, config: Value) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file: f,
            config_info: Arc::new(Some(config)),
            extra_info: None,
            work_dir: None,
        };
        CommonOutput.execute(&ctx)
    }

    #[test]
    fn fails_without_file() {
        let out = run(None, json!({}));
        assert!(matches!(out.result, OutputResultType::Failed));
        assert!(out.message.contains("未获得可访问 URL"));
    }

    #[test]
    fn fails_for_local_file() {
        let out = run(
            Some(file(FileDataType::FilePath, "/tmp/a.png")),
            json!({}),
        );
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn fails_for_non_http_url() {
        let out = run(
            Some(file(FileDataType::NetworkPath, "ftp://x/a.png")),
            json!({}),
        );
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn defaults_to_markdown_and_preserves_file() {
        let out = run(
            Some(file(FileDataType::NetworkPath, "https://x/a.png")),
            json!({}),
        );
        assert_eq!(out.message, "![a.png](https://x/a.png)");
        assert_eq!(out.file.unwrap().input_path, "https://x/a.png");
        let info = out.extra_info.unwrap();
        assert_eq!(info.get("output_format").unwrap(), "markdown");
        assert_eq!(info.get("output").unwrap(), "![a.png](https://x/a.png)");
    }

    #[test]
    fn template_overrides_format_and_sets_template_label() {
        let out = run(
            Some(file(FileDataType::NetworkPath, "https://x/a.png")),
            json!({"format": "html", "template": "{url}"}),
        );
        assert_eq!(out.message, "https://x/a.png");
        assert_eq!(
            out.extra_info.unwrap().get("output_format").unwrap(),
            "template"
        );
    }

    #[test]
    fn link_format_returns_plain_url() {
        let out = run(
            Some(file(FileDataType::NetworkPath, "https://x/a.png")),
            json!({"format": "link"}),
        );
        assert_eq!(out.message, "https://x/a.png");
        assert_eq!(
            out.extra_info.unwrap().get("output_format").unwrap(),
            "link"
        );
    }

    #[test]
    fn validate_rejects_unknown_format() {
        let ctx = UploadInputCtx {
            file: None,
            config_info: Arc::new(Some(json!({"format": "pdf"}))),
            extra_info: None,
            work_dir: None,
        };
        assert!(CommonOutput.validate_params(&ctx).is_err());
    }

    #[test]
    fn validate_accepts_known_formats() {
        for f in ["markdown", "html", "link"] {
            let ctx = UploadInputCtx {
                file: None,
                config_info: Arc::new(Some(json!({"format": f}))),
                extra_info: None,
                work_dir: None,
            };
            assert!(CommonOutput.validate_params(&ctx).is_ok());
        }
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        CommonOutput.on_load();
        CommonOutput.on_unload();
    }

    #[test]
    fn config_json_passes_declarative_validation() {
        use file_uploader_sdk::models::config_schema::PluginConfigInfo;
        use file_uploader_sdk::utils::validate_util::validate_plugin_config;
        let raw = include_str!("../../resources/output/common_output/config.json");
        let info: PluginConfigInfo =
            serde_json::from_str(raw).expect("config.json must parse into schema");
        validate_plugin_config(&info, &json!({})).expect("empty config validates");
        validate_plugin_config(&info, &json!({"format": "markdown", "template": "{url}"}))
            .expect("typical config validates");
    }
}
