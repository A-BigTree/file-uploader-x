mod naming;
mod multipart;
mod provider;
mod r2;
mod sigv4;

use std::collections::HashMap;
use std::sync::Arc;

use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{FileDataType, OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;

use naming::{build_object_key, sha256_file, NamingStrategy};
use provider::{StorageProvider, UploadMode, UploadRequest, UploadedObject};
use r2::R2Provider;

const DEFAULT_TIMEOUT_SECS: i64 = 30;
const DEFAULT_RETRY_TIMES: i64 = 2;
const DEFAULT_PART_SIZE: u64 = 64 * 1024 * 1024;
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_PART_COUNT: u64 = 10000;

pub struct CommonUploader;

struct UploadOptions {
    naming: NamingStrategy,
    key_prefix: String,
    multipart_enabled: bool,
    part_size: u64,
    timeout_secs: u64,
    retry_times: u32,
    account_id: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
    public_base_url: String,
    overwrite: bool,
}

impl UploadOptions {
    fn from_ctx(ctx: &UploadInputCtx) -> Result<Self, UploadError> {
        let naming_str = config_util::get_str(&ctx.config_info, "naming")
            .unwrap_or_else(|| "date_uuid".to_string());
        let naming = NamingStrategy::parse(&naming_str)?;
        let key_prefix = config_util::get_str(&ctx.config_info, "key_prefix").unwrap_or_default();
        let multipart_enabled =
            config_util::get_bool(&ctx.config_info, "multipart_enabled").unwrap_or(true);
        let part_size = config_util::get_size(&ctx.config_info, "multipart_part_size")
            .unwrap_or(DEFAULT_PART_SIZE);
        let timeout_secs =
            config_util::get_i64(&ctx.config_info, "timeout_secs").unwrap_or(DEFAULT_TIMEOUT_SECS)
                as u64;
        let retry_times =
            config_util::get_i64(&ctx.config_info, "retry_times").unwrap_or(DEFAULT_RETRY_TIMES)
                as u32;
        let account_id = config_util::get_str(&ctx.config_info, "account_id").unwrap_or_default();
        let bucket = config_util::get_str(&ctx.config_info, "bucket").unwrap_or_default();
        let access_key_id =
            config_util::get_str(&ctx.config_info, "access_key_id").unwrap_or_default();
        let secret_access_key =
            config_util::get_str(&ctx.config_info, "secret_access_key").unwrap_or_default();
        let public_base_url =
            config_util::get_str(&ctx.config_info, "public_base_url").unwrap_or_default();
        let overwrite = config_util::get_bool(&ctx.config_info, "overwrite").unwrap_or(true);
        Ok(Self {
            naming,
            key_prefix,
            multipart_enabled,
            part_size,
            timeout_secs,
            retry_times,
            account_id,
            bucket,
            access_key_id,
            secret_access_key,
            public_base_url,
            overwrite,
        })
    }
}

fn select_mode(multipart_enabled: bool, size: u64, part_size: u64) -> UploadMode {
    if multipart_enabled && size > part_size {
        UploadMode::Multipart { part_size }
    } else {
        UploadMode::Single
    }
}

fn build_success_output(
    orig: Arc<UploadFileData>,
    local_path: &str,
    provider: &str,
    uploaded: UploadedObject,
) -> UploadOutputCtx {
    let new_file = UploadFileData::new(
        FileDataType::NetworkPath,
        uploaded.url.clone(),
        orig.id.clone(),
        orig.name.clone(),
        orig.file_type.clone(),
        orig.size,
    );
    let mut extra: HashMap<String, String> = HashMap::new();
    extra.insert("upload_local_path".to_string(), local_path.to_string());
    extra.insert("object_key".to_string(), uploaded.object_key);
    extra.insert("provider".to_string(), provider.to_string());
    UploadOutputCtx {
        result: OutputResultType::Success,
        message: format!("common_uploader: uploaded to {}", uploaded.url),
        file: Some(Arc::new(new_file)),
        extra_info: Some(extra),
    }
}

fn validate_options(ctx: &UploadInputCtx) -> Result<(), UploadError> {
    let group = config_util::get_group(&ctx.config_info);
    if group.as_deref() != Some("r2") {
        return Err(UploadError::PluginParamInvalid(format!(
            "common_uploader 仅支持 group=r2，got {group:?}"
        )));
    }
    if let Some(url) = config_util::get_str(&ctx.config_info, "public_base_url") {
        let trimmed = url.trim();
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return Err(UploadError::PluginParamInvalid(
                "public_base_url 必须以 http:// 或 https:// 开头".into(),
            ));
        }
        if trimmed != url {
            return Err(UploadError::PluginParamInvalid(
                "public_base_url 不得含前后空白".into(),
            ));
        }
    }
    if let Some(prefix) = config_util::get_str(&ctx.config_info, "key_prefix") {
        if prefix.starts_with('/') {
            return Err(UploadError::PluginParamInvalid(
                "key_prefix 不得以 / 开头".into(),
            ));
        }
        if prefix.contains("..") {
            return Err(UploadError::PluginParamInvalid(
                "key_prefix 不得包含 ..".into(),
            ));
        }
    }
    let naming_str = config_util::get_str(&ctx.config_info, "naming")
        .unwrap_or_else(|| "date_uuid".to_string());
    NamingStrategy::parse(&naming_str)?;
    if let Some(ps) = config_util::get_size(&ctx.config_info, "multipart_part_size") {
        if ps < MIN_PART_SIZE || ps > MAX_PART_SIZE {
            return Err(UploadError::PluginParamInvalid(format!(
                "multipart_part_size {ps} 不在 5MB..5GB 范围"
            )));
        }
    }
    Ok(())
}

impl UploadPlugin for CommonUploader {
    fn name(&self) -> &'static str {
        "common_uploader"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::Upload
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        match self.run(ctx) {
            Ok(o) => o,
            Err(e) => UploadOutputCtx::failed(format!("common_uploader: {e}")),
        }
    }

    fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
        validate_options(ctx).map_err(|e| e.to_string())
    }

    fn on_load(&self) {
        tracing::info!("common_uploader: loading");
    }

    fn on_unload(&self) {
        tracing::info!("common_uploader: unloading");
    }
}

impl CommonUploader {
    fn run(&self, ctx: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        let options = UploadOptions::from_ctx(ctx)?;
        let f = ctx
            .file
            .as_ref()
            .ok_or_else(|| UploadError::PluginParamInvalid("no file to upload".into()))?
            .clone();
        let work_dir = ctx
            .work_dir
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(UploadError::WorkDirNotSet)?;
        let local_path = f.input_path.clone();

        let mode = select_mode(options.multipart_enabled, f.size as u64, options.part_size);
        if let UploadMode::Multipart { part_size } = mode {
            let parts = (f.size as u64).div_ceil(part_size);
            if parts > MAX_PART_COUNT {
                return Err(UploadError::InvalidFormat(format!(
                    "file requires {parts} parts, max {MAX_PART_COUNT}"
                )));
            }
        }

        let need_full_hash =
            matches!(mode, UploadMode::Single) || options.naming.needs_full_hash();
        let full_hash_owned: Option<String> = if need_full_hash {
            Some(sha256_file(work_dir, &local_path)?)
        } else {
            None
        };

        let date = chrono::Local::now().date_naive();
        let key = build_object_key(
            options.naming,
            &options.key_prefix,
            &f.name,
            full_hash_owned.as_deref(),
            date,
        )?;

        let content_type = if f.file_type.is_empty() {
            "application/octet-stream".to_string()
        } else {
            f.file_type.clone()
        };

        let provider = R2Provider::new(
            options.account_id,
            options.bucket,
            options.access_key_id,
            options.secret_access_key,
            options.public_base_url,
            options.overwrite,
            options.timeout_secs,
            options.retry_times,
        )?;

        let request = UploadRequest {
            work_dir,
            local_path: &local_path,
            object_key: &key,
            content_type: &content_type,
            size: f.size as u64,
            full_hash: full_hash_owned.as_deref(),
            mode,
        };
        let uploaded = provider.upload(&request)?;
        let provider_id = provider.provider_id();
        Ok(build_success_output(f, &local_path, provider_id, uploaded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::enums::OutputResultType;
    use serde_json::{json, Value};

    fn ctx_with(config: Value) -> UploadInputCtx {
        UploadInputCtx {
            file: None,
            config_info: Arc::new(Some(config)),
            extra_info: None,
            work_dir: None,
        }
    }

    fn file() -> Arc<UploadFileData> {
        Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            "/work/a.png".into(),
            "id1".into(),
            "a.png".into(),
            "image/png".into(),
            12,
        ))
    }

    #[test]
    fn default_options_use_date_uuid_and_64_mb_parts() {
        let ctx = ctx_with(json!({"group":"r2"}));
        let options = UploadOptions::from_ctx(&ctx).unwrap();
        assert_eq!(options.naming, NamingStrategy::DateUuid);
        assert!(options.multipart_enabled);
        assert_eq!(options.part_size, 64 * 1024 * 1024);
    }

    #[test]
    fn size_equal_to_part_size_uses_single_put() {
        assert_eq!(select_mode(true, 64, 64), UploadMode::Single);
    }

    #[test]
    fn size_above_part_size_uses_multipart() {
        assert_eq!(
            select_mode(true, 65, 64),
            UploadMode::Multipart { part_size: 64 }
        );
    }

    #[test]
    fn disabled_multipart_keeps_large_file_single() {
        assert_eq!(select_mode(false, 65, 64), UploadMode::Single);
    }

    #[test]
    fn uploaded_file_is_network_path_and_preserves_metadata() {
        let output = build_success_output(
            file(),
            "/work/a.png",
            "r2",
            UploadedObject {
                url: "https://cdn.example.com/a.png".into(),
                object_key: "a.png".into(),
            },
        );
        let out_file = output.file.unwrap();
        assert!(matches!(out_file.data_type, FileDataType::NetworkPath));
        assert_eq!(out_file.input_path, "https://cdn.example.com/a.png");
        assert_eq!(out_file.name, "a.png");
        assert_eq!(
            output.extra_info.unwrap().get("upload_local_path").unwrap(),
            "/work/a.png"
        );
    }

    #[test]
    fn execute_without_file_fails() {
        let ctx = ctx_with(json!({"group":"r2"}));
        let out = CommonUploader.execute(&ctx);
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn validate_rejects_non_r2_group() {
        let ctx = ctx_with(json!({"group":"oss"}));
        assert!(CommonUploader.validate_params(&ctx).is_err());
    }

    #[test]
    fn validate_rejects_missing_group() {
        let ctx = ctx_with(json!({}));
        assert!(CommonUploader.validate_params(&ctx).is_err());
    }

    #[test]
    fn validate_rejects_bad_public_base_url() {
        let ctx = ctx_with(json!({"group":"r2","public_base_url":"ftp://x"}));
        assert!(CommonUploader.validate_params(&ctx).is_err());
    }

    #[test]
    fn validate_rejects_prefix_with_slash_or_parent() {
        assert!(CommonUploader
            .validate_params(&ctx_with(json!({"group":"r2","key_prefix":"/a"})))
            .is_err());
        assert!(CommonUploader
            .validate_params(&ctx_with(json!({"group":"r2","key_prefix":"a/../b"})))
            .is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_part_size() {
        assert!(CommonUploader
            .validate_params(&ctx_with(
                json!({"group":"r2","multipart_part_size":"1MB"})
            ))
            .is_err());
    }

    #[test]
    fn validate_accepts_minimal_r2_config() {
        let ctx = ctx_with(json!({"group":"r2","public_base_url":"https://cdn.example.com"}));
        assert!(CommonUploader.validate_params(&ctx).is_ok());
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        CommonUploader.on_load();
        CommonUploader.on_unload();
    }

    #[test]
    fn config_json_passes_declarative_validation_for_complete_r2() {
        use file_uploader_sdk::models::config_schema::PluginConfigInfo;
        use file_uploader_sdk::utils::validate_util::validate_plugin_config;
        let raw = include_str!("../../resources/upload/common_uploader/config.json");
        let info: PluginConfigInfo =
            serde_json::from_str(raw).expect("config.json must parse into schema");
        let values = json!({
            "group": "r2",
            "account_id": "acct",
            "bucket": "b",
            "access_key_id": "ak",
            "secret_access_key": "sk",
            "public_base_url": "https://cdn.example.com"
        });
        validate_plugin_config(&info, &values).expect("complete r2 config validates");
    }
}
