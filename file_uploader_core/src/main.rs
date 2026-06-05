mod config;

use config::init_logging;
use file_uploader_sdk::error::UploadError;
use tracing::{info, warn};

struct FileInfo {
    size: u64,
    file_type: String,
    content: Vec<u8>,
}

fn validate_file(file: &FileInfo) -> Result<(), UploadError> {
    const MAX_SIZE: u64 = 1024 * 1024 * 10;
    const ALLOWED_TYPES: &[&str] = &["jpg", "png", "pdf"];

    if file.size > MAX_SIZE {
        return Err(UploadError::FileTooLarge {
            size: file.size,
            max: MAX_SIZE,
        });
    }

    if !ALLOWED_TYPES.contains(&file.file_type.as_str()) {
        return Err(UploadError::UnsupportedFileType {
            found: file.file_type.clone(),
            allowed: ALLOWED_TYPES.iter().map(|s| s.to_string()).collect(),
        });
    }

    if file.content.is_empty() {
        return Err(UploadError::InvalidFormat(
            "File content is empty".to_string(),
        ));
    }

    Ok(())
}

fn main() {
    // init_logging;
    if let Err(e) = init_logging() {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }

    let test_files = vec![
        FileInfo {
            size: 1024 * 1024 * 20,
            file_type: "jpg".to_string(),
            content: vec![1, 2, 3],
        },
        FileInfo {
            size: 1024,
            file_type: "exe".to_string(),
            content: vec![1, 2, 3],
        },
        FileInfo {
            size: 1024,
            file_type: "pdf".to_string(),
            content: vec![],
        },
    ];

    for file in test_files {
        match validate_file(&file) {
            Ok(_) => info!("File {:?} is valid", file.file_type),
            Err(e) => warn!("Validation failed for {:?}: {}", file.file_type, e),
        }
    }

    info!("Hello, world!")
}
