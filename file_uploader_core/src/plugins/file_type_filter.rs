use tracing::info;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::UploadPlugin;

struct FileTypeFilter;

impl UploadPlugin for FileTypeFilter {
    fn name(&self) -> &'static str {
        "file-type-filter"
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        info!("file-type-filter: {}", serde_json::to_string(ctx).unwrap_or("input error".to_string()));
        UploadOutputCtx {
            result: OutputResultType::Success,
            message: "Success".to_string(),
            file_list: None,
            extra_info: None
        }
    }
}