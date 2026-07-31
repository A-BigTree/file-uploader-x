use file_uploader_sdk::error::UploadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadMode {
    Single,
    Multipart { part_size: u64 },
}

pub struct UploadRequest<'a> {
    pub work_dir: &'a str,
    pub local_path: &'a str,
    pub object_key: &'a str,
    pub content_type: &'a str,
    pub size: u64,
    pub full_hash: Option<&'a str>,
    pub mode: UploadMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedObject {
    pub url: String,
    pub object_key: String,
}

pub trait StorageProvider {
    fn provider_id(&self) -> &'static str;
    fn upload(&self, request: &UploadRequest<'_>) -> Result<UploadedObject, UploadError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_mode_carries_part_size() {
        assert_eq!(
            UploadMode::Multipart {
                part_size: 64 * 1024 * 1024
            },
            UploadMode::Multipart {
                part_size: 64 * 1024 * 1024
            }
        );
    }
}
