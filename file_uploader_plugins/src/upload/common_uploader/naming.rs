use chrono::{Datelike, NaiveDate};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::utils::fs_util;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingStrategy {
    Origin,
    Uuid,
    Hash,
    DateUuid,
}

impl NamingStrategy {
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        match value {
            "origin" => Ok(Self::Origin),
            "uuid" => Ok(Self::Uuid),
            "hash" => Ok(Self::Hash),
            "date_uuid" => Ok(Self::DateUuid),
            other => Err(UploadError::InvalidFormat(format!(
                "unknown naming strategy: {other}"
            ))),
        }
    }

    pub fn needs_full_hash(self) -> bool {
        matches!(self, Self::Hash)
    }
}

pub fn sha256_file(work_dir: &str, path: &str) -> Result<String, UploadError> {
    let mut reader = fs_util::open_read(work_dir, path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn sanitize_name(name: &str) -> String {
    let no_parent = name.replace("..", "_");
    no_parent
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

pub fn build_object_key(
    strategy: NamingStrategy,
    prefix: &str,
    original_name: &str,
    full_hash: Option<&str>,
    date: NaiveDate,
) -> Result<String, UploadError> {
    let normalized_prefix = normalize_prefix(prefix);
    let ext = extension_of(original_name);

    let name = match strategy {
        NamingStrategy::Origin => sanitize_name(original_name),
        NamingStrategy::Uuid => fs_util::gen_unique_name(&ext),
        NamingStrategy::DateUuid => {
            let date_path = format!("{:04}/{:02}/{:02}", date.year(), date.month(), date.day());
            format!("{date_path}/{}", fs_util::gen_unique_name(&ext))
        }
        NamingStrategy::Hash => {
            let hash = full_hash.ok_or_else(|| {
                UploadError::InvalidFormat("hash naming requires full_hash".into())
            })?;
            if hash.len() < 16 {
                return Err(UploadError::InvalidFormat(format!(
                    "hash too short for naming: {} chars (need >= 16)",
                    hash.len()
                )));
            }
            let short = &hash[..16];
            if ext.is_empty() {
                short.to_string()
            } else {
                format!("{short}.{ext}")
            }
        }
    };

    Ok(format!("{normalized_prefix}{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use file_uploader_sdk::utils::fs_util;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 31).unwrap()
    }

    #[test]
    fn sanitize_origin_removes_path_and_parent_segments() {
        assert_eq!(sanitize_name("../a\\b\0.png"), "__a_b_.png");
    }

    #[test]
    fn origin_joins_normalized_prefix() {
        let key = build_object_key(NamingStrategy::Origin, "uploads/", "../demo.png", None, d())
            .unwrap();
        assert_eq!(key, "uploads/__demo.png");
    }

    #[test]
    fn date_uuid_uses_date_path_and_keeps_extension() {
        let key = build_object_key(NamingStrategy::DateUuid, "images/", "demo.png", None, d())
            .unwrap();
        assert!(key.starts_with("images/2026/07/31/"));
        assert!(key.ends_with(".png"));
    }

    #[test]
    fn hash_requires_and_truncates_full_hash() {
        let key = build_object_key(
            NamingStrategy::Hash,
            "",
            "demo.bin",
            Some("0123456789abcdefaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            d(),
        )
        .unwrap();
        assert_eq!(key, "0123456789abcdef.bin");
    }

    #[test]
    fn hash_rejects_missing_and_invalid_full_hashes() {
        assert!(build_object_key(NamingStrategy::Hash, "", "demo.bin", None, d()).is_err());
        assert!(build_object_key(
            NamingStrategy::Hash,
            "",
            "demo.bin",
            Some("0123456789"),
            d(),
        )
        .is_err());
    }

    #[test]
    fn generated_names_use_sanitized_extensions() {
        let key = build_object_key(NamingStrategy::Uuid, "p/", "arc.hive.tar", None, d()).unwrap();
        assert!(key.starts_with("p/"));
        assert!(key.ends_with(".tar"));
    }

    #[test]
    fn parser_rejects_unknown_strategy() {
        assert!(NamingStrategy::parse("random").is_err());
    }

    #[test]
    fn sha256_file_hashes_full_file_as_stream() {
        let tmp =
            std::env::temp_dir().join(format!("naming_sha_{}", fs_util::gen_unique_name("")));
        let work_dir = fs_util::create_work_dir(&tmp, "wd").unwrap();
        let work_dir_str = work_dir.to_str().unwrap();
        let (name, _path) = fs_util::write(work_dir_str, "bin", &b"abc"[..]).unwrap();
        assert_eq!(
            sha256_file(work_dir_str, &name).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}
