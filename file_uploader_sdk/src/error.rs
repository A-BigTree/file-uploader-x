use thiserror::Error;

#[derive(Debug, Error)]
pub enum UploadError {
    ///Common Error
    #[error("IO error: {0}")]
    CommonIoError(#[from] std::io::Error),

    #[error("Json serialize error")]
    JsonSerializeError(#[from] serde_json::Error),

    ///Param Error
    #[error("File size ({size} byte) is over {max} (byte)")]
    FileTooLarge { size: u64, max: u64 },

    #[error("File type ({found}) is not supported. Allowed types are: {allowed:?}")]
    UnsupportedFileType { found: String, allowed: Vec<String> },

    #[error("File format is invalid: {0}")]
    InvalidFormat(String),

    /// Plugin Error
    #[error("Plugin load error: {0}")]
    PluginLoadError(String),

    /// WorkDir / 沙箱错误
    #[error("work_dir is not set on context")]
    WorkDirNotSet,

    #[error("path '{path}' escapes work_dir '{work_dir}'")]
    WorkDirPathEscape { work_dir: String, path: String },
}

#[cfg(test)]
mod tests {
    use super::UploadError;

    #[test]
    fn work_dir_not_set_display() {
        let e = UploadError::WorkDirNotSet;
        let s = format!("{}", e);
        assert!(s.to_lowercase().contains("work_dir"), "got: {s}");
        assert!(s.to_lowercase().contains("not set"), "got: {s}");
    }

    #[test]
    fn work_dir_path_escape_display_carries_context() {
        let e = UploadError::WorkDirPathEscape {
            work_dir: "/data/wd".to_string(),
            path: "/etc/passwd".to_string(),
        };
        let s = format!("{}", e);
        assert!(s.contains("/data/wd"), "got: {s}");
        assert!(s.contains("/etc/passwd"), "got: {s}");
    }
}
