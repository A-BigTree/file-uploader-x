# work_dir 活动目录 + SDK 共性工具抽取 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `UploadInputCtx` 新增流程级 `work_dir` 活动目录并以强制沙箱 `fs_util` 约束插件文件 IO；抽取 `config_util` 与 `UploadOutputCtx` 关联函数等共性工具，改造 `file_type_filter` 作为范例，同步更新进程内插件 skill。

**Architecture:** 方案 A 自由函数沙箱——`work_dir: Option<String>` 流程级透传；SDK `utils/fs_util` 提供创建/校验/流式读写（写强制唯一名，预留自定义生成器）与 access 预留点 `check()`；`utils/config_util` 提供配置读取；`UploadOutputCtx` 增关联函数构造输出。stabby 层同步增字段（破坏性 ABI，须重编）。

**Tech Stack:** Rust 2024 / stabby ABI / serde / thiserror / chrono / tracing。无新外部依赖（fs_util 仅用 std + 已有 chrono）。

**规格依据：** `docs/superpowers/specs/2026-06-17-work-dir-sandbox-and-sdk-utils-design.md`

**关键约定：**
- `fs_util` 公开函数签名统一 `work_dir: &str`；**空串视为「未设置」** → 返回 `UploadError::WorkDirNotSet`。调用方从 `ctx.work_dir: Option<String>` 取值时用 `ctx.work_dir.as_deref().unwrap_or("")`，故 `None` 自然触发 `WorkDirNotSet`。
- `resolve` 用词法规范化（components 拼接，**不依赖 `canonicalize`**，避免写入时文件不存在导致校验失败）。
- 每个任务末尾提交；提交信息用 conventional commits。

**验证命令速查：**
- 单 crate 测试：`cargo test -p file_uploader_sdk`、`cargo test -p file_uploader_core`、`cargo test -p file_uploader_plugins`
- 全量：`cargo build` && `cargo test`

---

## Task 1: UploadError 新增沙箱错误变体

**Files:**
- Modify: `file_uploader_sdk/src/error.rs`

- [ ] **Step 1: 写失败测试（追加到 `error.rs` 的 `#[cfg(test)] mod tests`，若不存在则新建）**

在 `file_uploader_sdk/src/error.rs` 末尾追加：

```rust
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
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_sdk error::tests`
Expected: 编译失败，`no variant named WorkDirNotSet / WorkDirPathEscape`

- [ ] **Step 3: 实现两个变体**

在 `UploadError` enum 中（`PluginLoadError(String)` 之后）新增：

```rust
    /// WorkDir / 沙箱错误
    #[error("work_dir is not set on context")]
    WorkDirNotSet,

    #[error("path '{path}' escapes work_dir '{work_dir}'")]
    WorkDirPathEscape { work_dir: String, path: String },
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p file_uploader_sdk error::tests`
Expected: 2 passed

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/error.rs
git commit -m "feat(sdk): add WorkDirNotSet and WorkDirPathEscape error variants"
```

---

## Task 2: UploadOutputCtx 输出构造关联函数

**Files:**
- Modify: `file_uploader_sdk/src/models/ctx.rs`

- [ ] **Step 1: 写失败测试（追加到 `ctx.rs` 末尾）**

```rust
#[cfg(test)]
mod output_helper_tests {
    use super::*;
    use crate::models::enums::OutputResultType;

    #[test]
    fn success_has_no_files() {
        let o = UploadOutputCtx::success("ok");
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.message, "ok");
        assert!(o.file_list.is_none());
        assert!(o.extra_info.is_none());
    }

    #[test]
    fn success_files_carries_files() {
        let f = Arc::new(UploadFileData::new(
            crate::models::enums::FileDataType::FilePath,
            "/tmp/a".into(),
            "a".into(),
            "a".into(),
            "image/png".into(),
            0,
        ));
        let o = UploadOutputCtx::success_files("done", vec![f]);
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn failed_sets_failed_result() {
        let o = UploadOutputCtx::failed("boom");
        assert!(matches!(o.result, OutputResultType::Failed));
        assert_eq!(o.message, "boom");
        assert!(o.file_list.is_none());
    }

    #[test]
    fn interrupt_sets_interrupt_result() {
        let o = UploadOutputCtx::interrupt("stop");
        assert!(matches!(o.result, OutputResultType::Interrupt));
        assert_eq!(o.message, "stop");
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_sdk models::ctx::output_helper_tests`
Expected: 编译失败，`no function named success / failed / interrupt / success_files`

- [ ] **Step 3: 实现关联函数**

在 `ctx.rs` 的 `UploadOutputCtx` 结构体定义之后（`UploadOutputCtx` 当前无 `impl`，新增一个）：

```rust
impl UploadOutputCtx {
    /// 成功（无文件产出）
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file_list: None,
            extra_info: None,
        }
    }

    /// 成功（携带处理后文件）
    pub fn success_files(msg: impl Into<String>, files: Vec<Arc<UploadFileData>>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file_list: Some(files),
            extra_info: None,
        }
    }

    /// 失败（会中断 pipeline）
    pub fn failed(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Failed,
            message: msg.into(),
            file_list: None,
            extra_info: None,
        }
    }

    /// 中断
    pub fn interrupt(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Interrupt,
            message: msg.into(),
            file_list: None,
            extra_info: None,
        }
    }
}
```

> `OutputResultType` 已在 `ctx.rs` 顶部 `use crate::models::enums::{...}` 中导入，确认未缺；若缺则补 `OutputResultType`。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p file_uploader_sdk models::ctx::output_helper_tests`
Expected: 4 passed

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/models/ctx.rs
git commit -m "feat(sdk): add UploadOutputCtx success/failed/interrupt constructors"
```

---

## Task 3: config_util 配置读取 helper

**Files:**
- Create: `file_uploader_sdk/src/utils/config_util.rs`
- Modify: `file_uploader_sdk/src/utils.rs`

- [ ] **Step 1: 注册模块（先改 utils.rs 以便能引用）**

修改 `file_uploader_sdk/src/utils.rs` 为：

```rust
pub mod config_util;
pub mod ctx_util;
```

- [ ] **Step 2: 写失败测试（新建文件，含测试）**

创建 `file_uploader_sdk/src/utils/config_util.rs`：

```rust
use serde_json::Value;
use std::sync::Arc;

/// 从 config_info（Arc<Option<Value>>）读取某 key 的字符串值。
pub fn get_str(config: &Arc<Option<Value>>, key: &str) -> Option<String> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 读取布尔值。
pub fn get_bool(config: &Arc<Option<Value>>, key: &str) -> Option<bool> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
}

/// 读取字符串数组（非数组 / 元素非字符串均跳过）。缺失或 None → 空 Vec。
pub fn get_list(config: &Arc<Option<Value>>, key: &str) -> Vec<String> {
    config
        .as_ref()
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: &str) -> Arc<Option<Value>> {
        Arc::new(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn get_str_present() {
        let c = cfg(r#"{"k":"v"}"#);
        assert_eq!(get_str(&c, "k"), Some("v".to_string()));
    }

    #[test]
    fn get_str_missing_or_none() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert_eq!(get_str(&none_cfg, "k"), None);
        let c = cfg(r#"{"other":1}"#);
        assert_eq!(get_str(&c, "k"), None);
    }

    #[test]
    fn get_str_wrong_type() {
        let c = cfg(r#"{"k":123}"#);
        assert_eq!(get_str(&c, "k"), None);
    }

    #[test]
    fn get_bool_present() {
        let c = cfg(r#"{"flag":true}"#);
        assert_eq!(get_bool(&c, "flag"), Some(true));
    }

    #[test]
    fn get_bool_wrong_type() {
        let c = cfg(r#"{"flag":"yes"}"#);
        assert_eq!(get_bool(&c, "flag"), None);
    }

    #[test]
    fn get_list_present() {
        let c = cfg(r#"{"items":["a","b"]}"#);
        assert_eq!(get_list(&c, "items"), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn get_list_missing_returns_empty() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert!(get_list(&none_cfg, "items").is_empty());
        let c = cfg(r#"{"other":1}"#);
        assert!(get_list(&c, "items").is_empty());
    }

    #[test]
    fn get_list_non_string_elements_skipped() {
        let c = cfg(r#"{"items":["a", 1, "b"]}"#);
        assert_eq!(get_list(&c, "items"), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn get_list_not_array_returns_empty() {
        let c = cfg(r#"{"items":"x"}"#);
        assert!(get_list(&c, "items").is_empty());
    }
}
```

- [ ] **Step 3: 运行测试验证通过（实现已同文件给出）**

Run: `cargo test -p file_uploader_sdk utils::config_util`
Expected: 9 passed

- [ ] **Step 4: 提交**

```bash
git add file_uploader_sdk/src/utils.rs file_uploader_sdk/src/utils/config_util.rs
git commit -m "feat(sdk): add config_util helpers for reading typed config values"
```

---

## Task 4: fs_util 基础（gen_unique_name / create_work_dir / resolve）

**Files:**
- Create: `file_uploader_sdk/src/utils/fs_util.rs`
- Modify: `file_uploader_sdk/src/utils.rs`

- [ ] **Step 1: 注册模块**

修改 `file_uploader_sdk/src/utils.rs` 为：

```rust
pub mod config_util;
pub mod ctx_util;
pub mod fs_util;
```

- [ ] **Step 2: 写失败测试 + 实现基础部分（新建文件）**

创建 `file_uploader_sdk/src/utils/fs_util.rs`：

```rust
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
}
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test -p file_uploader_sdk utils::fs_util`
Expected: 6 passed

- [ ] **Step 4: 提交**

```bash
git add file_uploader_sdk/src/utils.rs file_uploader_sdk/src/utils/fs_util.rs
git commit -m "feat(sdk): add fs_util base (gen_unique_name, create_work_dir, resolve sandbox)"
```

---

## Task 5: fs_util 写入（write / write_with_gen 流式 + 唯一名）

**Files:**
- Modify: `file_uploader_sdk/src/utils/fs_util.rs`

- [ ] **Step 1: 写失败测试（追加到 `fs_util.rs` 的 `mod tests`）**

在 `fs_util.rs` 的 `#[cfg(test)] mod tests` 内追加：

```rust
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
        let (name, path) = write_with_gen(tmp.to_str().unwrap(), "txt", b"x"[..], my_gen).unwrap();
        assert_eq!(name, "custom-fixed.txt");
        assert!(path.ends_with("custom-fixed.txt"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_with_empty_work_dir_is_not_set() {
        let err = write("", "txt", b"x"[..]).unwrap_err();
        assert!(matches!(err, UploadError::WorkDirNotSet));
    }
```

> 注意 `&data[..]` 与 `b"x"[..]` 是 `impl Read` 的切片形式。

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_sdk utils::fs_util::tests::write`
Expected: 编译失败，`cannot find function write / write_with_gen`

- [ ] **Step 3: 实现 write / write_with_gen**

在 `fs_util.rs`（`resolve` 函数之后、`#[cfg(test)]` 之前）新增，并补充 `use std::io::Read;` 到文件顶部 `use` 区：

```rust
use std::io::Read;
```

新增函数：

```rust
/// 写文件（默认生成器）：从任意 `Read` 流式拷贝（`io::copy` 分块）到 work_dir 内唯一名文件。
/// 返回 `(文件名, 完整路径)`。`work_dir` 为空 → `WorkDirNotSet`。
pub fn write(work_dir: &str, ext: &str, reader: impl Read) -> Result<(String, PathBuf), UploadError> {
    write_with_gen(work_dir, ext, reader, gen_unique_name)
}

/// 写文件（自定义生成器 `gen`）：同上，但文件名由 `gen` 决定（预留扩展点）。
pub fn write_with_gen(
    work_dir: &str,
    ext: &str,
    mut reader: impl Read,
    gen: fn(&str) -> String,
) -> Result<(String, PathBuf), UploadError> {
    let name = gen(ext);
    let path = resolve(work_dir, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&path)?;
    std::io::copy(&mut reader, &mut file)?;
    Ok((name, path))
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p file_uploader_sdk utils::fs_util`
Expected: 9 passed（基础 6 + 写入 3）

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/utils/fs_util.rs
git commit -m "feat(sdk): add fs_util streaming write with mandatory unique name"
```

---

## Task 6: fs_util 读取与通用（open_read / read_to_end / read_to_string / exists / create_dir）

**Files:**
- Modify: `file_uploader_sdk/src/utils/fs_util.rs`

- [ ] **Step 1: 写失败测试（追加到 `mod tests`）**

```rust
    #[test]
    fn open_read_returns_stream() {
        let tmp = std::env::temp_dir().join(format!("fxutil_or_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let (name, _) = write(tmp.to_str().unwrap(), "txt", b"stream-me"[..]).unwrap();
        let mut r = open_read(tmp.to_str().unwrap(), &name).unwrap();
        let mut buf = String::new();
        use std::io::Read;
        r.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "stream-me");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn read_to_end_and_read_to_string_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("fxutil_rte_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let (name, _) = write(tmp.to_str().unwrap(), "txt", b"abc"[..]).unwrap();
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
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_sdk utils::fs_util::tests::open_read utils::fs_util::tests::read_to_end utils::fs_util::tests::exists`
Expected: 编译失败，`cannot find function open_read / read_to_end / read_to_string / exists / create_dir`

- [ ] **Step 3: 实现读取与通用函数**

在 `fs_util.rs`（`write_with_gen` 之后、`#[cfg(test)]` 之前）新增：

```rust
use std::io::BufReader;

/// 打开 work_dir 内文件，返回 `BufReader<File>`（流式，调用方自行读取）。
pub fn open_read(work_dir: &str, filename: &str) -> Result<BufReader<std::fs::File>, UploadError> {
    let path = resolve(work_dir, filename)?;
    let f = std::fs::File::open(&path)?;
    Ok(BufReader::new(f))
}

/// 便捷：把 work_dir 内文件一次性读为 `Vec<u8>`（基于 `open_read`）。
pub fn read_to_end(work_dir: &str, filename: &str) -> Result<Vec<u8>, UploadError> {
    use std::io::Read;
    let mut r = open_read(work_dir, filename)?;
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)?;
    Ok(buf)
}

/// 便捷：把 work_dir 内文件一次性读为 `String`（基于 `open_read`）。
pub fn read_to_string(work_dir: &str, filename: &str) -> Result<String, UploadError> {
    use std::io::Read;
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
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p file_uploader_sdk utils::fs_util`
Expected: 13 passed（6 + 3 + 4）

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/utils/fs_util.rs
git commit -m "feat(sdk): add fs_util streaming read helpers and exists/create_dir"
```

---

## Task 7: work_dir 字段穿透 + pipeline 透传

> 此任务为一次性「字段穿透」：增字段会让所有 `UploadInputCtx { .. }` 与 `UploadInputCtxS { .. }` 字面量构造编译失败，必须同步补齐全部构造点。透传逻辑（`output_to_input` / `plugin_input` 取 `source_ctx.work_dir.clone()`）一并落地。

**Files:**
- Modify: `file_uploader_sdk/src/models/ctx.rs`
- Modify: `file_uploader_sdk/src/models/ctx_stabby.rs`
- Modify: `file_uploader_sdk/src/utils/ctx_util.rs`
- Modify: `file_uploader_core/src/pipeline/registry.rs`（含 `output_to_input`、`plugin_input`、测试构造点）
- Modify: `file_uploader_plugins/src/pre_upload/file_type_filter.rs`（仅测试 `run()` 补字段）
- Modify: `file_uploader_core/src/main.rs`

- [ ] **Step 1: 写失败测试——ctx_util 含 work_dir 往返（追加到 `ctx_util.rs` 末尾）**

```rust
#[cfg(test)]
mod work_dir_tests {
    use super::*;
    use crate::models::ctx::{UploadFileData};
    use crate::models::enums::FileDataType;

    fn input_with_work_dir(wd: Option<&str>) -> UploadInputCtx {
        UploadInputCtx {
            file_list: vec![Arc::new(UploadFileData::new(
                FileDataType::FilePath,
                "/tmp/a".into(),
                "a".into(),
                "a".into(),
                "image/png".into(),
                0,
            ))],
            config_info: Arc::new(None),
            extra_info: None,
            related_process_info: None,
            work_dir: wd.map(String::from),
        }
    }

    #[test]
    fn roundtrip_preserves_work_dir_some() {
        let original = input_with_work_dir(Some("/data/wd-1"));
        let s = convert_input_ctx_s(&original);
        let back = convert_input_ctx(&s);
        assert_eq!(back.work_dir.as_deref(), Some("/data/wd-1"));
    }

    #[test]
    fn roundtrip_preserves_work_dir_none() {
        let original = input_with_work_dir(None);
        let s = convert_input_ctx_s(&original);
        let back = convert_input_ctx(&s);
        assert!(back.work_dir.is_none());
    }
}
```

> 此时编译会因 `work_dir` 字段不存在而失败——这是预期的「红」。

- [ ] **Step 2: 增字段（ctx.rs）**

在 `UploadInputCtx` 结构体（`related_process_info` 之后）新增字段：

```rust
    /// 活动目录：本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传）。
    /// None 表示未设置。#[serde(default)] 兼容旧 JSON。
    #[serde(default)]
    pub work_dir: Option<String>,
```

- [ ] **Step 3: 增字段（ctx_stabby.rs）**

在 `UploadInputCtxS`（`extra_info` 之后）新增：

```rust
    // 活动目录
    pub work_dir: SOption<SString>,
```

- [ ] **Step 4: 双向转换（ctx_util.rs）**

`convert_input_ctx_s` 返回结构体（line 55-59 附近）补 `work_dir` 字段：

```rust
    UploadInputCtxS {
        file_list,
        config_info,
        extra_info,
        work_dir: match &input.work_dir {
            None => None.into(),
            Some(s) => SOption::Some(s.clone().into()),
        },
    }
```

`convert_input_ctx`（line 80-88 附近）补 `work_dir`：

```rust
    UploadInputCtx {
        file_list,
        config_info: input.config_info.match_ref(
            |config_info_s| Arc::new(get_config(config_info_s)),
            || Arc::new(None),
        ),
        extra_info,
        related_process_info: None,
        work_dir: input.work_dir.match_ref(|s| Some(s.clone().into()), || None),
    }
```

> `SOption::match_ref` 在同文件已有用法（见 `extra_info` 还原），保持一致。

- [ ] **Step 5: registry 透传（registry.rs）**

`output_to_input`（line 162-183）返回结构体补字段：

```rust
        UploadInputCtx {
            file_list: output.file_list.clone().unwrap_or_default(),
            config_info: Arc::new(None),
            extra_info: if extra_info.is_empty() {
                None
            } else {
                Some(extra_info)
            },
            related_process_info: source_ctx.related_process_info.clone(),
            work_dir: source_ctx.work_dir.clone(),
        }
```

`execute_pipeline` 内 `plugin_input` 构造（line 222-227）补字段：

```rust
                let plugin_input = UploadInputCtx {
                    file_list: current_ctx.file_list.clone(),
                    config_info: Arc::new(plugin.registry_config.clone()),
                    extra_info: current_ctx.extra_info.clone(),
                    related_process_info: current_ctx.related_process_info.clone(),
                    work_dir: current_ctx.work_dir.clone(),
                };
```

- [ ] **Step 6: 补齐 registry.rs 测试构造点**

在 `registry.rs` 测试中所有 `UploadInputCtx { file_list: vec![], config_info: Arc::new(None), extra_info: None, related_process_info: None }` 字面量补 `work_dir: None,`。涉及测试函数（搜索 `related_process_info: None` 定位）：
- `test_execute_pipeline_empty_registry`
- `test_execute_pipeline_single_plugin_callback_order`
- `test_execute_pipeline_plugin_failed_interrupts`
- `test_execute_pipeline_registry_config_injected`

每个字面量在 `related_process_info: None,` 后追加一行 `work_dir: None,`。

- [ ] **Step 7: 补齐 file_type_filter.rs 测试构造点**

`file_uploader_plugins/src/pre_upload/file_type_filter.rs` 的 `run()`（line 127-134）补字段：

```rust
        let ctx = UploadInputCtx {
            file_list: files,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir: None,
        };
```

- [ ] **Step 8: 补齐 main.rs 构造点**

`file_uploader_core/src/main.rs`（line 19-24）补字段：

```rust
    let ctx = UploadInputCtx {
        file_list: vec![],
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
        work_dir: None,
    };
```

- [ ] **Step 9: 编译并运行测试验证通过**

Run: `cargo build`
Expected: 成功（无构造点遗漏）

Run: `cargo test -p file_uploader_sdk utils::ctx_util::work_dir_tests`
Expected: 2 passed

- [ ] **Step 10: 新增 pipeline 透传测试（追加到 registry.rs 测试模块）**

在 `registry.rs` 的 `#[cfg(test)] mod tests` 内新增（复用已有的 `create_mock_plugin_info` 等 helper）：

```rust
    #[test]
    fn test_work_dir_propagates_to_plugin_and_across_output_to_input() {
        // 用 ConfigReadPlugin 模式：捕获 plugin_input 的 work_dir
        struct CaptureWorkDir {
            seen: Arc<Mutex<Option<String>>>,
        }
        impl file_uploader_sdk::models::interface::UploadPlugin for CaptureWorkDir {
            fn name(&self) -> &'static str { "capture_wd" }
            fn phase(&self) -> UploadPhase { UploadPhase::Upload }
            fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
                *self.seen.lock().unwrap() = ctx.work_dir.clone();
                UploadOutputCtx {
                    result: file_uploader_sdk::models::enums::OutputResultType::Success,
                    message: "ok".into(),
                    file_list: Some(ctx.file_list.clone()),
                    extra_info: None,
                }
            }
        }
        let seen = Arc::new(Mutex::new(None));
        let plugin = Arc::new(CaptureWorkDir { seen: seen.clone() });
        let meta = Arc::new(PluginMeta {
            name: "capture_wd".to_string(),
            title: "Capture".to_string(),
            version: "1.0.0".to_string(),
            description: "capture work_dir".to_string(),
            author: None,
            phase: UploadPhase::Upload,
        });
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test".to_string(),
                plugin: plugin as Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let info = Arc::new(UploadPluginInfo {
            id: "test_capture_wd".to_string(),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test".to_string(),
            slot,
        });
        let reg = PluginRegistryInfo::new(info, 1, PluginRegistryStatus::Enable, None);
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);

        let input = UploadInputCtx {
            file_list: vec![],
            config_info: Arc::new(None),
            extra_info: None,
            related_process_info: None,
            work_dir: Some("/data/wd-flow".to_string()),
        };
        table.execute_pipeline(input, None);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/data/wd-flow"));
    }
```

- [ ] **Step 11: 运行全部 core 测试验证通过**

Run: `cargo test -p file_uploader_core`
Expected: 全部通过（含新透传测试）

- [ ] **Step 12: 提交**

```bash
git add file_uploader_sdk/src/models/ctx.rs file_uploader_sdk/src/models/ctx_stabby.rs file_uploader_sdk/src/utils/ctx_util.rs file_uploader_core/src/pipeline/registry.rs file_uploader_plugins/src/pre_upload/file_type_filter.rs file_uploader_core/src/main.rs
git commit -m "feat: thread work_dir through UploadInputCtx/stabby/ctx_util and pipeline"
```

---

## Task 8: file_type_filter 改造（采用 config_util + 输出关联函数）

**Files:**
- Modify: `file_uploader_plugins/src/pre_upload/file_type_filter.rs`

- [ ] **Step 1: 运行既有测试建立基线**

Run: `cargo test -p file_uploader_plugins pre_upload::file_type_filter`
Expected: 现有 6 个测试全部通过（回归基线）

- [ ] **Step 2: 改造实现**

将 `file_type_filter.rs` 的 helper 区（`parse_list` / `parse_config`）替换为使用 `config_util`，输出构造改用 `UploadOutputCtx` 关联函数。具体改动：

a) 调整 imports（文件顶部）：

```rust
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;
use glob::Pattern;
use std::sync::Arc;
use tracing::{info, warn};
```
（移除 `use serde_json::Value;`，因不再直接用 Value）

b) 删除 `parse_list` 与 `parse_config` 两个函数，替换为：

```rust
/// 解析 pass_type / reject_type。config_info 为 None → 双空（全部允许）。
fn parse_config(config_info: &Arc<Option<serde_json::Value>>) -> (Vec<String>, Vec<String>) {
    (
        config_util::get_list(config_info, "pass_type"),
        config_util::get_list(config_info, "reject_type"),
    )
}
```

c) `execute` 内的两处 `UploadOutputCtx { ... }` 字面量改用关联函数：

失败分支（`filtered.is_empty()`）：

```rust
        if filtered.is_empty() {
            return UploadOutputCtx::failed(format!(
                "file_type_filter: all {} file(s) rejected (pass={:?}, reject={:?})",
                total, pass_raw, reject_raw
            ));
        }

        UploadOutputCtx::success_files(
            format!("file_type_filter: {}/{} passed", passed, total),
            filtered,
        )
```

（`info!` 日志行中 `serde_json::to_string(ctx)` 保持不变，`ctx` 仍可序列化；`serde_json` 仍由该行使用，故保留 `serde_json` 在作用域——`parse_config` 签名已显式写 `serde_json::Value`，无需顶层 `use Value`。）

- [ ] **Step 3: 运行测试验证回归通过**

Run: `cargo test -p file_uploader_plugins pre_upload::file_type_filter`
Expected: 现有 6 个测试全部通过（行为不变）

- [ ] **Step 4: 提交**

```bash
git add file_uploader_plugins/src/pre_upload/file_type_filter.rs
git commit -m "refactor(plugins): file_type_filter uses config_util and output constructors"
```

---

## Task 9: 同步更新 designing-in-process-plugins skill

**Files:**
- Modify: `.agents/skills/designing-in-process-plugins/SKILL.md`

- [ ] **Step 1: 改「配置读取约定」（第 3 节）**

将原第 3 节内容替换为：

```markdown
### 3. 配置读取约定

运行期配置经 `registry_config → ctx.config_info`（裸 `serde_json::Value`）注入。结构约定为 `{ "<param_key>": <default_value 同型值>, ... }`，与 `config.json` params 的 key 对齐。`config_info` 为 `None` → 用默认/空，插件须容忍。

**优先使用 SDK helper**（`file_uploader_sdk::utils::config_util`），不要手写 `Value` 解析：

- `config_util::get_str(&ctx.config_info, "key") -> Option<String>`
- `config_util::get_bool(&ctx.config_info, "key") -> Option<bool>`
- `config_util::get_list(&ctx.config_info, "key") -> Vec<String>`（数组→字符串列表，缺失/非数组→空）
```

- [ ] **Step 2: 改「输出约定」（第 4 节）**

将原第 4 节内容替换为：

```markdown
### 4. 输出约定

**优先使用 `UploadOutputCtx` 关联函数**构造输出，避免手写字面量：

- 正常：`UploadOutputCtx::success_files(msg, files)`（`success(msg)` 无文件产出）
- 过滤/校验类插件，若处理后列表为空 → `UploadOutputCtx::failed(msg)`（会中断 pipeline）
- 中断：`UploadOutputCtx::interrupt(msg)`
- `extra_info` 需传递时仍可先构造再赋值字段

行为约定：正常 `result = Success`、`message` 记统计；空列表用 `Failed`；`extra_info: Option<HashMap<String,String>>` 可累积传递给下游插件。
```

- [ ] **Step 3: 新增「活动目录与文件 IO」小节（插入到第 4 节之后、第 5 节「模块注册」之前）**

```markdown
### 5. 活动目录与文件 IO

`ctx.work_dir: Option<String>` 是本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传，`None`=未设置）。插件需读写中间文件时**必须走 `file_uploader_sdk::utils::fs_util`**，**禁止直接用 `std::fs`**：

- 写文件：`fs_util::write(ctx.work_dir.as_deref().unwrap_or(""), ext, reader) -> Result<(文件名, 路径)>` —— **强制使用生成的唯一文件名**（`{ts}_{hash}.{ext}`），不支持调用方指定文件名；自定义命名用 `fs_util::write_with_gen(.., gen: fn(&str)->String)`。
- 读文件：`fs_util::open_read(work_dir, filename) -> Result<BufReader<File>>`（流式）；便捷：`read_to_end` / `read_to_string`。
- 其它：`fs_util::create_work_dir(base, id)`（宿主侧创建活动目录）、`fs_util::resolve`（沙箱路径校验）、`fs_util::exists` / `fs_util::create_dir`。

错误：`work_dir` 为空/`None` 调 IO → `UploadError::WorkDirNotSet`；`../` 或绝对路径越界 → `WorkDirPathEscape`。

**强制力边界**：dylib 插件直接调 libc / `std::fs` 无法被拦截，沙箱仅覆盖「走 `fs_util`」的路径；`BufReader<File>` 为 std 类型，不跨 dylib ABI 边界传递。未来会结合 `config.json` 的 `access`（fs_read/fs_write）与 work_dir 做权限收敛（当前预留未实现）。
```

（后续小节编号顺延：原「模块注册」变第 6、「测试」变第 7、「注册到 pipeline」变第 8。同步更新正文中对这些编号的引用，如无则跳过。）

- [ ] **Step 4: 改「测试」（测试构造点补 work_dir）**

将原第 6 节（现第 7 节）测试段中关于构造 `UploadInputCtx` 的描述补充一句：

```markdown
构造 `UploadInputCtx` 时须补 `work_dir` 字段（不涉及文件 IO 的插件测试用 `None`；涉及者用临时目录）。
```

- [ ] **Step 5: 改「范例」段**

在 `file_type_filter` 范例描述后补一句：

```markdown
该插件已改造为使用 `config_util::get_list` 与 `UploadOutputCtx::failed/success_files`，可作为新工具用法的范例。
```

- [ ] **Step 6: 改「常见错误」表（新增三行）**

在常见错误表格末尾追加：

```markdown
| `WorkDirNotSet` | `ctx.work_dir` 为 `None`（或空串）时调用 `fs_util` IO；先确认宿主已透传 work_dir |
| `WorkDirPathEscape` | 传入 `../` 或绝对路径越出 work_dir；仅用 `fs_util` 生成的唯一名或在 work_dir 内的相对路径 |
| dylib 加载/运行 ABI 不兼容 | stabby 结构体增字段为破坏性 ABI 变更，宿主与 dylib 须同版本重编（重新 `cargo build -p uploader_example_plugin`） |
```

- [ ] **Step 7: 改「快速检查清单」（增补四项）**

在清单末尾追加：

```markdown
- [ ] 读配置走 `config_util`（不手写 `Value` 解析）
- [ ] 构造输出走 `UploadOutputCtx` 关联函数（success/failed/interrupt/success_files）
- [ ] 读写中间文件走 `fs_util`（不直接用 `std::fs`）
- [ ] 测试构造 `UploadInputCtx` 时补 `work_dir` 字段
```

- [ ] **Step 8: 提交**

```bash
git add .agents/skills/designing-in-process-plugins/SKILL.md
git commit -m "docs(skill): sync in-process plugin skill with work_dir and sdk utils"
```

---

## Task 10: 全量验证

**Files:** 无（仅验证）

- [ ] **Step 1: 全量构建**

Run: `cargo build`
Expected: 成功，无警告新增（如有与本次无关的旧警告可忽略）

- [ ] **Step 2: 全量测试**

Run: `cargo test`
Expected: 全部通过

- [ ] **Step 3: 重编示例 dylib（验证 ABI 变更后链路通）**

Run: `cargo build -p uploader_example_plugin`
Expected: 成功

- [ ] **Step 4: 运行核心示例（验证 in-process + dylib 端到端）**

Run: `cargo run -p file_uploader_core`
Expected: 输出 `Logging initialized`、`=== Testing IN-PROCESS plugin ===`、插件加载与执行结果、`=== Testing DYLIB plugin WITH logger ===`、dylib 执行结果，无 `error!` 中断。

- [ ] **Step 5: 最终提交（如有 lint/格式修正）**

```bash
git add -A
git commit -m "chore: full workspace verification for work_dir and sdk utils" || echo "nothing to commit"
```

---

## 完成判据

- `UploadInputCtx` / `UploadInputCtxS` 含 `work_dir`，双向转换与 pipeline 全程透传。
- `fs_util`（创建/校验/流式读写/唯一名/自定义生成器）、`config_util`、`UploadOutputCtx` 关联函数全部就绪并有测试覆盖。
- `file_type_filter` 改造完成、既有测试回归通过。
- `designing-in-process-plugins` skill 同步更新。
- `cargo build` && `cargo test` 全绿；`cargo run -p file_uploader_core` 端到端正常。
