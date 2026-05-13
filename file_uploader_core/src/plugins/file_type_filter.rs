use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::UploadPlugin;

struct FileTypeFilter;

impl UploadPlugin for FileTypeFilter {
    fn name(&self) -> &'static str {
        "file-type-filter"
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        UploadOutputCtx {
            result: OutputResultType::Success,
            message: "Success".to_string(),
            file_list: None,
            extra_info: None
        }
    }
}