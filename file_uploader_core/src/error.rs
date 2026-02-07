use thiserror::Error;

#[derive(Debug, Error)]
pub enum UploadError {
    // Param Error
    #[error("File size ({size} byte) is over {max} (byte)")]
    FileTooLarge {size: u64, max: u64},

    #[error("")]
    UnsupportedFileType { found: String, allowed: Vec<String> },
}