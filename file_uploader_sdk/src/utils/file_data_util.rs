//! 文件数据统一读取：Binary（内存字节）/ FilePath（落盘路径）两种形态的桥接工具。
//!
//! 语义约定：`UploadFileData.data_type == Binary` 时 `data` 字段为权威数据，
//! `input_path` 仅保留原始来源路径作标识/展示，不保证可读。

use std::io::Cursor;
use std::path::PathBuf;

use crate::error::UploadError;
use crate::models::ctx::UploadFileData;
use crate::models::enums::FileDataType;

/// 统一读取文件内容：Binary → `data` 字段；FilePath → 读盘；NetworkPath → 报错（应先经 Input 阶段）。
pub fn read_file_data(f: &UploadFileData) -> Result<Vec<u8>, UploadError> {
    match f.data_type {
        FileDataType::Binary => f
            .data
            .as_ref()
            .map(|d| d.as_slice().to_vec())
            .ok_or_else(|| {
                UploadError::PluginParamInvalid(format!(
                    "binary file '{}' has no data payload",
                    f.name
                ))
            }),
        FileDataType::FilePath => std::fs::read(&f.input_path).map_err(|e| {
            UploadError::PluginParamInvalid(format!("read '{}': {e}", f.input_path))
        }),
        FileDataType::NetworkPath => Err(UploadError::PluginParamInvalid(format!(
            "network path '{}' must be handled by an Input-phase plugin first",
            f.input_path
        ))),
    }
}

/// 降级出口：Binary 时把字节落盘到 work_dir 唯一名文件并改写为 FilePath；FilePath 原样返回路径。
/// 供需要文件路径的插件（如 dylib 插件）在内存流程中按需降级。
pub fn ensure_file_path(f: &mut UploadFileData, work_dir: &str) -> Result<PathBuf, UploadError> {
    match f.data_type {
        FileDataType::FilePath => Ok(PathBuf::from(&f.input_path)),
        FileDataType::Binary => {
            let data = read_file_data(f)?;
            let ext = f
                .name
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            let (_name, path) = super::fs_util::write(work_dir, &ext, Cursor::new(data))?;
            f.data_type = FileDataType::FilePath;
            f.input_path = path.to_string_lossy().into_owned();
            f.data = None;
            Ok(path)
        }
        FileDataType::NetworkPath => Err(UploadError::PluginParamInvalid(format!(
            "network path '{}' must be handled by an Input-phase plugin first",
            f.input_path
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::enums::FileDataType;
    use stabby::sync::Arc as SArc;
    use stabby::vec::Vec as SVec;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fdu_{tag}_{}_{}",
            std::process::id(),
            crate::utils::fs_util::gen_unique_name("")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_binary_returns_data() {
        let f = UploadFileData::binary(
            "/orig/x.png".into(),
            "id1".into(),
            "x.png".into(),
            "image/png".into(),
            vec![1u8, 2, 3],
        );
        assert_eq!(read_file_data(&f).unwrap(), vec![1u8, 2, 3]);
    }

    #[test]
    fn read_binary_without_data_errors() {
        let mut f = UploadFileData::binary(
            "/orig/x.png".into(),
            "id1".into(),
            "x.png".into(),
            "image/png".into(),
            vec![1u8],
        );
        f.data = None;
        assert!(read_file_data(&f).is_err());
    }

    #[test]
    fn read_file_path_reads_disk() {
        let dir = tmp_dir("read");
        let p = dir.join("a.bin");
        std::fs::write(&p, b"abc").unwrap();
        let f = UploadFileData::new(
            FileDataType::FilePath,
            p.display().to_string(),
            "id1".into(),
            "a.bin".into(),
            String::new(),
            3,
        );
        assert_eq!(read_file_data(&f).unwrap(), b"abc".to_vec());
    }

    #[test]
    fn read_network_errors() {
        let f = UploadFileData::new(
            FileDataType::NetworkPath,
            "https://x/a.png".into(),
            "id1".into(),
            "a.png".into(),
            String::new(),
            0,
        );
        assert!(read_file_data(&f).is_err());
    }

    #[test]
    fn binary_construction_sets_size_and_data() {
        let f = UploadFileData::binary(
            "/o/b".into(),
            "i".into(),
            "b".into(),
            "x/y".into(),
            vec![9u8; 12],
        );
        assert!(matches!(f.data_type, FileDataType::Binary));
        assert_eq!(f.size, 12);
        let d = f.data.as_ref().unwrap();
        let expected: SArc<SVec<u8>> = SArc::new(SVec::from([9u8; 12].as_slice()));
        assert_eq!(d.as_slice(), expected.as_slice());
    }

    #[test]
    fn ensure_file_path_passes_through_file_path() {
        let dir = tmp_dir("passthrough");
        let p = dir.join("b.txt");
        std::fs::write(&p, b"zz").unwrap();
        let mut f = UploadFileData::new(
            FileDataType::FilePath,
            p.display().to_string(),
            "i".into(),
            "b.txt".into(),
            String::new(),
            2,
        );
        let got = ensure_file_path(&mut f, dir.to_str().unwrap()).unwrap();
        assert_eq!(got, p);
    }

    #[test]
    fn ensure_file_path_degrades_binary_to_disk() {
        let dir = tmp_dir("degrade");
        let mut f = UploadFileData::binary(
            "/o/c.png".into(),
            "i".into(),
            "c.png".into(),
            "image/png".into(),
            vec![7u8, 8],
        );
        let path = ensure_file_path(&mut f, dir.to_str().unwrap()).unwrap();
        assert!(path.starts_with(&dir));
        assert!(path.to_string_lossy().ends_with(".png"));
        assert!(matches!(f.data_type, FileDataType::FilePath));
        assert!(f.data.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), vec![7u8, 8]);
        // 降级后 read_file_data 走磁盘路径
        assert_eq!(read_file_data(&f).unwrap(), vec![7u8, 8]);
    }

    #[test]
    fn ensure_file_path_binary_empty_work_dir_errors() {
        let mut f = UploadFileData::binary(
            "/o/d".into(),
            "i".into(),
            "d".into(),
            String::new(),
            vec![1u8],
        );
        assert!(ensure_file_path(&mut f, "").is_err());
    }
}
