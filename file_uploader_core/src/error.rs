use thiserror::Error;

#[derive(Debug, Error)]
pub enum UploadError {
    //! Param Error
    #[error("File size ({size} byte) is over {max} (byte)")]
    FileTooLarge {size: u64, max: u64},

    #[error("File type ({found}) is not supported. Allowed types are: {allowed:?}")]
    UnsupportedFileType { found: String, allowed: Vec<String> },

    #[error("File format is invalid: {0}")]
    InvalidFormat(String),
    
    //! IO Error
}