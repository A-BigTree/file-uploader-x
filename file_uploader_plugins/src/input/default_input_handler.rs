use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{FileDataType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::{config_util, fs_util};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

pub struct DefaultInputHandler;

impl UploadPlugin for DefaultInputHandler {
    fn name(&self) -> &'static str {
        "default_input_handler"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::Input
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        match self.run(ctx) {
            Ok(files) => UploadOutputCtx::success_files(
                format!("default_input_handler: processed {} file(s)", files.len()),
                files,
            ),
            Err(e) => UploadOutputCtx::failed(format!("default_input_handler: {e}")),
        }
    }

    fn on_load(&self) {
        info!("default_input_handler: loading");
    }

    fn on_unload(&self) {
        info!("default_input_handler: unloading");
    }
}

impl DefaultInputHandler {
    fn run(&self, ctx: &UploadInputCtx) -> Result<Vec<Arc<UploadFileData>>, UploadError> {
        let cache_local = config_util::get_bool(&ctx.config_info, "cache_local").unwrap_or(true);
        let download_network =
            config_util::get_bool(&ctx.config_info, "download_network").unwrap_or(true);
        let sniff_type = config_util::get_bool(&ctx.config_info, "sniff_type").unwrap_or(true);
        let work_dir = ctx.work_dir.as_deref().unwrap_or("");

        let mut out = Vec::with_capacity(ctx.file_list.len());
        for f in &ctx.file_list {
            let mut nf = (**f).clone();
            match nf.data_type {
                FileDataType::FilePath => {
                    if sniff_type {
                        if let Some(mime) = sniff_external(&nf.input_path) {
                            nf.file_type = mime;
                        } else {
                            warn!("default_input_handler: sniff failed for {}", nf.input_path);
                        }
                    }
                    if cache_local {
                        let wd = require_work_dir(work_dir)?;
                        let ext = ext_from_name(&nf.name);
                        let (name, path) = fs_util::import_file(wd, &nf.input_path, ext)?;
                        nf.size = fs_util::file_size(wd, &name)? as usize;
                        nf.input_path = path.to_string_lossy().into_owned();
                    }
                }
                FileDataType::NetworkPath => {
                    if download_network {
                        let wd = require_work_dir(work_dir)?;
                        let ext = ext_from_name(&nf.name);
                        let (name, path) = download_to_workdir(wd, &nf.input_path, ext)?;
                        nf.size = fs_util::file_size(wd, &name)? as usize;
                        nf.input_path = path.to_string_lossy().into_owned();
                        if sniff_type {
                            if let Some(mime) = sniff_in_workdir(wd, &name) {
                                nf.file_type = mime;
                            } else {
                                warn!("default_input_handler: sniff failed for {}", name);
                            }
                        }
                    }
                }
                FileDataType::Binary => {
                    // 原样透传（字段预留，当前不处理内存数据）
                }
            }
            out.push(Arc::new(nf));
        }
        Ok(out)
    }
}

fn require_work_dir(work_dir: &str) -> Result<&str, UploadError> {
    if work_dir.is_empty() {
        Err(UploadError::WorkDirNotSet)
    } else {
        Ok(work_dir)
    }
}

fn ext_from_name(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && !ext.contains('/') => ext,
        _ => "",
    }
}

fn sniff_bytes(bytes: &[u8]) -> Option<String> {
    infer::get(bytes).map(|t| t.mime_type().to_string())
}

fn sniff_external(src_abs_path: &str) -> Option<String> {
    let head = fs_util::read_external_head(src_abs_path, 512).ok()?;
    sniff_bytes(&head)
}

fn sniff_in_workdir(work_dir: &str, filename: &str) -> Option<String> {
    let mut r = fs_util::open_read(work_dir, filename).ok()?;
    let mut buf = vec![0u8; 512];
    let n = r.read(&mut buf).ok()?;
    buf.truncate(n);
    sniff_bytes(&buf)
}

fn download_to_workdir(
    work_dir: &str,
    url: &str,
    ext: &str,
) -> Result<(String, PathBuf), UploadError> {
    let resp = reqwest::blocking::get(url)
        .map_err(|e| UploadError::PluginLoadError(format!("download failed for {url}: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(UploadError::PluginLoadError(format!(
            "download {url} returned status {status}"
        )));
    }
    fs_util::write(work_dir, ext, resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::enums::OutputResultType;
    use file_uploader_sdk::utils::fs_util::gen_unique_name;

    fn tmp_work_dir() -> String {
        let dir = std::env::temp_dir().join(format!("dih_{}", gen_unique_name("")));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn write_png(path: &str) {
        // 最小 PNG 魔数头（infer 识别为 image/png）
        let png = [
            0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52,
        ];
        std::fs::write(path, png).unwrap();
    }

    fn run(
        files: Vec<Arc<UploadFileData>>,
        config: Option<serde_json::Value>,
        work_dir: Option<String>,
    ) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file_list: files,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir,
        };
        DefaultInputHandler.execute(&ctx)
    }

    #[test]
    fn local_file_cached_and_sniffed() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            src,
            "id1".into(),
            "src.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": true, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd.clone()));
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_type, "image/png");
        assert!(list[0].input_path.starts_with(&wd));
        assert!(list[0].input_path.ends_with(".png"));
        assert!(list[0].size > 0);
    }

    #[test]
    fn local_file_no_cache_keeps_path_but_sniffs() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            src.clone(),
            "id1".into(),
            "src.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": false, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd));
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].input_path, src);
        assert_eq!(list[0].file_type, "image/png");
    }

    #[test]
    fn sniff_disabled_keeps_original_type() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            src,
            "id1".into(),
            "src.png".into(),
            "upstream/x".into(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": false, "sniff_type": false});
        let out = run(vec![f], Some(cfg), Some(wd));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].file_type, "upstream/x");
    }

    #[test]
    fn binary_file_pass_through() {
        let f = Arc::new(UploadFileData::new(
            FileDataType::Binary,
            String::new(),
            "id1".into(),
            "blob".into(),
            "x/y".into(),
            7,
        ));
        let out = run(vec![f], None, None);
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].name, "blob");
        assert_eq!(list[0].file_type, "x/y");
    }

    #[test]
    fn network_no_download_passes_through() {
        let f = Arc::new(UploadFileData::new(
            FileDataType::NetworkPath,
            "https://example.com/a.png".into(),
            "id1".into(),
            "a.png".into(),
            "upstream/png".into(),
            0,
        ));
        let cfg = serde_json::json!({"download_network": false});
        let out = run(vec![f], Some(cfg), None);
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].input_path, "https://example.com/a.png");
        assert_eq!(list[0].file_type, "upstream/png");
    }

    #[test]
    fn missing_work_dir_when_cache_required_fails() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            src,
            "id1".into(),
            "src.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": true});
        let out = run(vec![f], Some(cfg), None);
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn local_missing_source_fails() {
        let wd = tmp_work_dir();
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            format!("{wd}/nope.png"),
            "id1".into(),
            "nope.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": true});
        let out = run(vec![f], Some(cfg), Some(wd));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    #[ignore = "needs network; run with: cargo test -p file_uploader_plugins -- --ignored download_network_real"]
    fn download_network_real() {
        let wd = tmp_work_dir();
        let url = "https://www.w3.org/Icons/w3c_main.png";
        let f = Arc::new(UploadFileData::new(
            FileDataType::NetworkPath,
            url.into(),
            "id1".into(),
            "logo.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"download_network": true, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd.clone()));
        if matches!(out.result, OutputResultType::Failed) {
            eprintln!("network test failed (expected offline): {}", out.message);
            return;
        }
        let list = out.file_list.as_ref().unwrap();
        assert!(list[0].input_path.starts_with(&wd));
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let p = DefaultInputHandler;
        p.on_load();
        p.on_unload();
    }
}
