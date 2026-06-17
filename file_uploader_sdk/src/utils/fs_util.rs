use crate::error::UploadError;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 默认文件名生成器：`{timestamp_ms}_{hash}.{ext}`
/// hash = DefaultHasher((timestamp_ms, 原子计数器)) 的十六进制。
pub fn gen_unique_name(ext: &str) -> String {
    let ts = chrono::Local::now().timestamp_millis();
    let n = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut h = DefaultHasher::new();
    (ts, n).hash(&mut h);
    let hash = h.finish();
    let ext_part = if ext.is_empty() {
        String::new()
    } else if ext.starts_with('.') {
        ext.to_string()
    } else {
        format!(".{}", ext)
    };
    format!("{}_{:x}{}", ts, hash, ext_part)
}

/// 创建活动目录：`base/<id>`，create_dir_all。返回完整路径供填入 ctx.work_dir。
pub fn create_work_dir(base: &Path, id: &str) -> Result<PathBuf, UploadError> {
    let dir = base.join(id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 词法规范化：处理 `.` / `..`，不依赖文件是否存在（不用 canonicalize）。
fn lex_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                _ => {}
            },
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

/// 沙箱校验扩展点：本期仅判「归属」（resolved 是否以 work_dir 为前缀）。
/// 未来在此追加 config.json access 白名单与 work_dir 的交集收敛。
fn check(work_dir: &Path, resolved: &Path) -> Result<(), UploadError> {
    if !resolved.starts_with(work_dir) {
        return Err(UploadError::WorkDirPathEscape {
            work_dir: work_dir.display().to_string(),
            path: resolved.display().to_string(),
        });
    }
    Ok(())
}

/// 沙箱核心：`join(rel)` → 词法规范化 → 校验仍以 work_dir 为前缀。
/// `work_dir` 为空串 → `WorkDirNotSet`；越界（`..`、绝对路径）→ `WorkDirPathEscape`。
pub fn resolve(work_dir: &str, rel: &str) -> Result<PathBuf, UploadError> {
    if work_dir.is_empty() {
        return Err(UploadError::WorkDirNotSet);
    }
    let base = lex_normalize(Path::new(work_dir));
    let normalized = lex_normalize(&base.join(rel));
    check(&base, &normalized)?;
    Ok(normalized)
}

use std::io::Read;

/// 写文件（默认生成器）：从任意 `Read` 流式拷贝（`io::copy` 分块）到 work_dir 内唯一名文件。
/// 返回 `(文件名, 完整路径)`。`work_dir` 为空 → `WorkDirNotSet`。
pub fn write(
    work_dir: &str,
    ext: &str,
    reader: impl Read,
) -> Result<(String, PathBuf), UploadError> {
    write_with_gen(work_dir, ext, reader, gen_unique_name)
}

/// 写文件（自定义生成器 `gen_fn`）：同上，但文件名由 `gen_fn` 决定（预留扩展点）。
pub fn write_with_gen(
    work_dir: &str,
    ext: &str,
    mut reader: impl Read,
    gen_fn: fn(&str) -> String,
) -> Result<(String, PathBuf), UploadError> {
    let name = gen_fn(ext);
    let path = resolve(work_dir, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&path)?;
    std::io::copy(&mut reader, &mut file)?;
    Ok((name, path))
}

use std::io::BufReader;

/// 打开 work_dir 内文件，返回 `BufReader<File>`（流式，调用方自行读取）。
pub fn open_read(
    work_dir: &str,
    filename: &str,
) -> Result<BufReader<std::fs::File>, UploadError> {
    let path = resolve(work_dir, filename)?;
    let f = std::fs::File::open(&path)?;
    Ok(BufReader::new(f))
}

/// 便捷：把 work_dir 内文件一次性读为 `Vec<u8>`（基于 `open_read`）。
pub fn read_to_end(work_dir: &str, filename: &str) -> Result<Vec<u8>, UploadError> {
    let mut r = open_read(work_dir, filename)?;
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    Ok(buf)
}

/// 便捷：把 work_dir 内文件一次性读为 `String`（基于 `open_read`）。
pub fn read_to_string(work_dir: &str, filename: &str) -> Result<String, UploadError> {
    let mut r = open_read(work_dir, filename)?;
    let mut s = String::new();
    r.read_to_string(&mut s)?;
    Ok(s)
}

/// work_dir 内某相对路径是否存在。
pub fn exists(work_dir: &str, rel: &str) -> bool {
    match resolve(work_dir, rel) {
        Ok(p) => p.exists(),
        Err(_) => false,
    }
}

/// 在 work_dir 内创建子目录（含前缀校验）。
pub fn create_dir(work_dir: &str, rel: &str) -> Result<(), UploadError> {
    let path = resolve(work_dir, rel)?;
    std::fs::create_dir_all(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_unique_name_is_unique_and_carries_ext() {
        let a = gen_unique_name("txt");
        let b = gen_unique_name("txt");
        assert_ne!(a, b);
        assert!(a.ends_with(".txt"));
        let none = gen_unique_name("");
        assert!(!none.ends_with('.'), "no trailing dot when ext empty");
        let with_dot = gen_unique_name(".json");
        assert!(with_dot.ends_with(".json"));
        assert!(!with_dot.ends_with("..json"));
    }

    #[test]
    fn create_work_dir_creates_nested() {
        let tmp = std::env::temp_dir().join(format!("fxutil_cwd_{}", gen_unique_name("")));
        let sub = create_work_dir(&tmp, "sub").unwrap();
        assert!(sub.is_dir());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_normal_relative() {
        let tmp = std::env::temp_dir().join(format!("fxutil_res_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let r = resolve(tmp.to_str().unwrap(), "a/b.txt").unwrap();
        assert_eq!(r, tmp.join("a/b.txt"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_empty_work_dir_is_not_set() {
        let err = resolve("", "x").unwrap_err();
        assert!(matches!(err, UploadError::WorkDirNotSet));
    }

    #[test]
    fn resolve_parent_dir_escape_rejected() {
        let tmp = std::env::temp_dir().join(format!("fxutil_esc_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let err = resolve(tmp.to_str().unwrap(), "../../etc/passwd").unwrap_err();
        assert!(matches!(err, UploadError::WorkDirPathEscape { .. }));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_absolute_path_inside_workdir_ok() {
        let tmp = std::env::temp_dir().join(format!("fxutil_abs_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let inside = tmp.join("c.txt");
        let r = resolve(tmp.to_str().unwrap(), inside.to_str().unwrap()).unwrap();
        assert_eq!(r, inside);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_uses_unique_name_and_streams_content() {
        let tmp = std::env::temp_dir().join(format!("fxutil_w_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let data = b"hello-bytes";
        let (name, path) = write(tmp.to_str().unwrap(), "bin", &data[..]).unwrap();
        assert!(name.ends_with(".bin"));
        assert!(path.starts_with(&tmp));
        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, data);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_with_gen_uses_custom_generator() {
        let tmp = std::env::temp_dir().join(format!("fxutil_wg_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        fn my_gen(ext: &str) -> String {
            format!("custom-{}.{}", "fixed", ext)
        }
        let (name, path) =
            write_with_gen(tmp.to_str().unwrap(), "txt", &b"x"[..], my_gen).unwrap();
        assert_eq!(name, "custom-fixed.txt");
        assert!(path.ends_with("custom-fixed.txt"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_with_empty_work_dir_is_not_set() {
        let err = write("", "txt", &b"x"[..]).unwrap_err();
        assert!(matches!(err, UploadError::WorkDirNotSet));
    }

    #[test]
    fn open_read_returns_stream() {
        use std::io::Read;
        let tmp = std::env::temp_dir().join(format!("fxutil_or_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let (name, _) = write(tmp.to_str().unwrap(), "txt", &b"stream-me"[..]).unwrap();
        let mut r = open_read(tmp.to_str().unwrap(), &name).unwrap();
        let mut buf = String::new();
        r.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "stream-me");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn read_to_end_and_read_to_string_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("fxutil_rte_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let (name, _) = write(tmp.to_str().unwrap(), "txt", &b"abc"[..]).unwrap();
        let v = read_to_end(tmp.to_str().unwrap(), &name).unwrap();
        assert_eq!(v, b"abc");
        let s = read_to_string(tmp.to_str().unwrap(), &name).unwrap();
        assert_eq!(s, "abc");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn exists_and_create_dir() {
        let tmp = std::env::temp_dir().join(format!("fxutil_ex_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!exists(tmp.to_str().unwrap(), "sub"));
        create_dir(tmp.to_str().unwrap(), "sub").unwrap();
        assert!(exists(tmp.to_str().unwrap(), "sub"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn read_functions_reject_escape() {
        let tmp = std::env::temp_dir().join(format!("fxutil_rej_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let err = read_to_end(tmp.to_str().unwrap(), "../../etc/passwd").unwrap_err();
        assert!(matches!(err, UploadError::WorkDirPathEscape { .. }));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
