# 进程内插件迭代实现计划（upload_file_filter 重构 + default_input_handler 新增）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构 `file_type_filter` 为三维度（类型+名称+大小）的 `upload_file_filter`；新增 Input 阶段 `default_input_handler` 插件（本地缓存 + 网络下载 + 魔数嗅探填充 `file_type`）。

**Architecture:** SDK 工具层先行（`config_util` 大小解析、`fs_util` 受控外部 IO 入口），再迁移/重构过滤器，最后实现 input 插件。所有插件改动遵循 `designing-in-process-plugins` 规范：资源目录(meta+config) + Rust 实现 + 模块注册 + build.rs 自动复制。`execute` 同步签名；任一文件失败即整体 `Failed` 中断。

**Tech Stack:** Rust edition 2024 / `glob`(过滤) / `infer`(魔数嗅探) / `reqwest` blocking(下载) / `stabby`(ABI) / `tracing`(日志) / TDD。

**Spec:** `docs/superpowers/specs/2026-06-19-in-process-plugin-iteration-design.md`

**测试命令约定：**
- 单 crate 测试：`cargo test -p <crate>`
- 类型/构建检查：`cargo build`
- 全量：`cargo build && cargo test`

---

## Task 1: `config_util` 新增 `parse_size` + `get_size`

**Files:**
- Modify: `file_uploader_sdk/src/utils/config_util.rs`

- [ ] **Step 1: 写失败测试**

在 `config_util.rs` 的 `#[cfg(test)] mod tests` 末尾追加：

```rust
    #[test]
    fn parse_size_plain_bytes() {
        assert_eq!(super::parse_size("1024"), Some(1024));
    }

    #[test]
    fn parse_size_units_binary() {
        assert_eq!(super::parse_size("1kb"), Some(1024));
        assert_eq!(super::parse_size("1mb"), Some(1024 * 1024));
        assert_eq!(super::parse_size("1gb"), Some(1024u64 * 1024 * 1024));
        assert_eq!(super::parse_size("1g"), Some(1024u64 * 1024 * 1024));
        assert_eq!(super::parse_size("1tb"), Some(1024u64 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_size_case_insensitive_and_trimmed() {
        assert_eq!(super::parse_size("  10MB "), Some(10 * 1024 * 1024));
        assert_eq!(super::parse_size("2Kb"), Some(2 * 1024));
    }

    #[test]
    fn parse_size_invalid() {
        assert_eq!(super::parse_size(""), None);
        assert_eq!(super::parse_size("abc"), None);
        assert_eq!(super::parse_size("1xb"), None);
    }

    #[test]
    fn get_size_from_string() {
        let c = cfg(r#"{"max":"2mb"}"#);
        assert_eq!(super::get_size(&c, "max"), Some(2 * 1024 * 1024));
    }

    #[test]
    fn get_size_from_number() {
        let c = cfg(r#"{"max":1048576}"#);
        assert_eq!(super::get_size(&c, "max"), Some(1048576));
    }

    #[test]
    fn get_size_missing_or_invalid() {
        let none_cfg: Arc<Option<Value>> = Arc::new(None);
        assert_eq!(super::get_size(&none_cfg, "max"), None);
        let c = cfg(r#"{"max":"abc"}"#);
        assert_eq!(super::get_size(&c, "max"), None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_sdk utils::config_util`
Expected: 编译失败（`parse_size` / `get_size` 未定义）

- [ ] **Step 3: 实现 `parse_size` 与 `get_size`**

在 `config_util.rs` 的 `get_list` 函数之后、`#[cfg(test)]` 之前插入：

```rust
/// 解析带单位的大小字符串为字节数（二进制 1024 进制）。
/// 支持：纯数字（按字节）或 数字+单位；单位大小写不敏感。
///   b/byte/bytes → 1；k/kb → 1024；m/mb → 1024²；g/gb → 1024³；t/tb → 1024⁴
/// 非法输入 → None。
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let split = s.find(|c: char| c.is_ascii_alphabetic());
    let (num_part, unit_part) = match split {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let num: u64 = num_part.parse().ok()?;
    let mult: u64 = match unit_part.to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024 * 1024,
        "g" | "gb" => 1024u64 * 1024 * 1024,
        "t" | "tb" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    num.checked_mul(mult)
}

/// 读取某 key 的大小：字符串走 `parse_size`，数字走 `as_u64`。缺失/无法解析 → None。
pub fn get_size(config: &Arc<Option<Value>>, key: &str) -> Option<u64> {
    let v = config.as_ref().as_ref()?.get(key)?;
    match v {
        Value::String(s) => parse_size(s),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_sdk utils::config_util`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/utils/config_util.rs
git commit -m "feat(sdk): add config_util parse_size/get_size for human-readable file sizes"
```

---

## Task 2: `fs_util` 新增 `import_file` + `read_external_head` + `file_size`

**Files:**
- Modify: `file_uploader_sdk/src/utils/fs_util.rs`

- [ ] **Step 1: 写失败测试**

在 `fs_util.rs` 的 `#[cfg(test)] mod tests` 末尾追加（沿用现有临时目录风格）：

```rust
    #[test]
    fn import_file_copies_external_into_sandbox() {
        let tmp = std::env::temp_dir().join(format!("fxutil_imp_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        // 外部源文件（放在 tmp 之外也行，此处简化放 tmp/src）
        let src = tmp.join("src.bin");
        std::fs::write(&src, b"hello-external").unwrap();
        let (name, path) = import_file(tmp.to_str().unwrap(), src.to_str().unwrap(), "bin").unwrap();
        assert!(name.ends_with(".bin"));
        assert!(path.starts_with(&tmp));
        assert_eq!(std::fs::read(&path).unwrap(), b"hello-external");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn import_file_missing_src_errors() {
        let tmp = std::env::temp_dir().join(format!("fxutil_impmiss_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let err = import_file(tmp.to_str().unwrap(), "/nonexistent/pathxyz", "bin").unwrap_err();
        assert!(matches!(err, UploadError::CommonIoError(_)));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn import_file_empty_work_dir_is_not_set() {
        let err = import_file("", "/tmp/x", "bin").unwrap_err();
        assert!(matches!(err, UploadError::WorkDirNotSet));
    }

    #[test]
    fn read_external_head_returns_prefix() {
        let tmp = std::env::temp_dir().join(format!("fxutil_head_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("data.dat");
        std::fs::write(&src, b"0123456789").unwrap();
        let head = read_external_head(src.to_str().unwrap(), 4).unwrap();
        assert_eq!(head, b"0123");
        // n 超过文件长度 → 返回实际读取长度
        let head2 = read_external_head(src.to_str().unwrap(), 100).unwrap();
        assert_eq!(head2, b"0123456789");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn read_external_head_missing_src_errors() {
        let err = read_external_head("/nonexistent/xyzabc", 4).unwrap_err();
        assert!(matches!(err, UploadError::CommonIoError(_)));
    }

    #[test]
    fn file_size_reads_sandbox_file_len() {
        let tmp = std::env::temp_dir().join(format!("fxutil_fs_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let (name, _) = write(tmp.to_str().unwrap(), "bin", &b"abcdef"[..]).unwrap();
        assert_eq!(file_size(tmp.to_str().unwrap(), &name).unwrap(), 6);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn file_size_rejects_escape() {
        let tmp = std::env::temp_dir().join(format!("fxutil_fse_{}", gen_unique_name("")));
        std::fs::create_dir_all(&tmp).unwrap();
        let err = file_size(tmp.to_str().unwrap(), "../../etc/passwd").unwrap_err();
        assert!(matches!(err, UploadError::WorkDirPathEscape { .. }));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_sdk utils::fs_util`
Expected: 编译失败（三个函数未定义）

- [ ] **Step 3: 实现三个函数**

在 `fs_util.rs` 的 `create_dir` 函数之后、`#[cfg(test)]` 之前插入：

```rust
/// 把外部绝对路径文件拷贝进 work_dir 唯一名文件（受控「外部→沙箱」入口）。
/// src_abs_path 不走 resolve 前缀校验（外部源），写入仍经 write（唯一名 + 沙箱）。
/// work_dir 为空 → WorkDirNotSet；src 不存在 → IO 错误。
pub fn import_file(
    work_dir: &str,
    src_abs_path: &str,
    ext: &str,
) -> Result<(String, PathBuf), UploadError> {
    let reader = std::fs::File::open(src_abs_path)?;
    write(work_dir, ext, reader)
}

/// 读取外部文件前 n 字节（供魔数嗅探）。不走 work_dir 沙箱校验。
/// 文件短于 n 字节 → 返回实际读取长度。
pub fn read_external_head(src_abs_path: &str, n: usize) -> Result<Vec<u8>, UploadError> {
    use std::io::Read;
    let mut f = std::fs::File::open(src_abs_path)?;
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf)?;
    buf.truncate(read);
    Ok(buf)
}

/// 读取 work_dir 内某文件大小（字节）。走 resolve 沙箱校验。
pub fn file_size(work_dir: &str, filename: &str) -> Result<u64, UploadError> {
    let path = resolve(work_dir, filename)?;
    Ok(std::fs::metadata(path)?.len())
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_sdk utils::fs_util`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```bash
git add file_uploader_sdk/src/utils/fs_util.rs
git commit -m "feat(sdk): add fs_util import_file/read_external_head/file_size for input plugin"
```

---

## Task 3: `UploadFileData` 派生 `Clone`（input 插件需修改文件副本）

**Files:**
- Modify: `file_uploader_sdk/src/models/ctx.rs:134`
- Modify: `file_uploader_sdk/src/models/ctx_stabby.rs`（注释）

- [ ] **Step 1: 给 `UploadFileData` 加 `Clone`**

在 `ctx.rs` 第 134 行，把：

```rust
#[derive(Serialize, Deserialize)]
pub struct UploadFileData {
```

改为：

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct UploadFileData {
```

- [ ] **Step 2: 更新 `file_type` 语义注释**

在 `ctx.rs` 第 144-145 行，把：

```rust
    // 文件类型
    pub file_type: String,
```

改为：

```rust
    // 文件类型（MIME，由 Input 阶段 default_input_handler 魔数嗅探填充；未嗅探时为上游原值）
    pub file_type: String,
```

在 `ctx_stabby.rs` 第 21 行附近，给 `file_type: SString` 加同样注释（保持 stabby ABI，仅语义说明）：

```rust
    // 文件类型（MIME，由 Input 阶段魔数嗅探填充）
    pub file_type: SString,
```

- [ ] **Step 3: 构建验证**

Run: `cargo build -p file_uploader_sdk`
Expected: 编译通过（stabby `SArc`/`SVec` 支持 `Clone`）

> 若编译失败提示 `SArc` 或 `SVec` 未实现 `Clone`：改为在 input 插件内用结构体字面量手动重建 `UploadFileData`（逐字段 clone，`data` 字段用 `f.data.clone()`，若仍失败则 `data: None` 并记为已知限制）。

- [ ] **Step 4: 提交**

```bash
git add file_uploader_sdk/src/models/ctx.rs file_uploader_sdk/src/models/ctx_stabby.rs
git commit -m "refactor(sdk): derive Clone on UploadFileData; document file_type MIME semantics"
```

---

## Task 4: `file_uploader_plugins` 加 `infer` + `reqwest` 依赖

**Files:**
- Modify: `file_uploader_plugins/Cargo.toml`

- [ ] **Step 1: 加依赖**

在 `file_uploader_plugins/Cargo.toml` 的 `[dependencies]` 末尾追加：

```toml
infer = "0.19"
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }
```

> 说明：`reqwest` 关闭 default-features，仅启用 `blocking`（同步下载）+ `rustls-tls`（纯 Rust TLS，免系统 OpenSSL 依赖）。`infer` 用于魔数嗅探。

- [ ] **Step 2: 构建验证依赖可拉取**

Run: `cargo build -p file_uploader_plugins`
Expected: 编译通过（首次会下载依赖）

- [ ] **Step 3: 提交**

```bash
git add file_uploader_plugins/Cargo.toml
git commit -m "build(plugins): add infer and reqwest(blocking) dependencies"
```

---

## Task 5: 迁移 `file_type_filter` 资源目录 → `upload_file_filter`

**Files:**
- Move: `file_uploader_plugins/resources/pre/file_type_filter/` → `file_uploader_plugins/resources/pre/upload_file_filter/`
- Modify: `.../upload_file_filter/meta.json`
- Modify: `.../upload_file_filter/config.json`

- [ ] **Step 1: 重命名资源目录**

```bash
git mv file_uploader_plugins/resources/pre/file_type_filter file_uploader_plugins/resources/pre/upload_file_filter
```

- [ ] **Step 2: 更新 `meta.json`**

把 `file_uploader_plugins/resources/pre/upload_file_filter/meta.json` 改为：

```json
{
  "name": "upload_file_filter",
  "title": "上传文件过滤器",
  "description": "按文件类型(MIME)、文件名、大小三维过滤上传文件",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "PreUpload"
}
```

- [ ] **Step 3: 更新 `config.json`（三维度配置）**

把 `file_uploader_plugins/resources/pre/upload_file_filter/config.json` 改为：

```json
{
  "access": {
    "fs_read": false,
    "fs_write": false,
    "network": false
  },
  "params": [
    {
      "key": "pass_type",
      "title": "允许类型",
      "description": "允许通过的文件类型(MIME)，为空表示全部允许，支持 glob 如 image/*",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true,
        "options": [
          {"label": "图片(image/*)", "value": "image/*"},
          {"label": "PNG", "value": "image/png"},
          {"label": "JPEG", "value": "image/jpeg"},
          {"label": "GIF", "value": "image/gif"},
          {"label": "PDF", "value": "application/pdf"},
          {"label": "纯文本", "value": "text/plain"},
          {"label": "视频(video/*)", "value": "video/*"},
          {"label": "音频(audio/*)", "value": "audio/*"},
          {"label": "ZIP 压缩包", "value": "application/zip"},
          {"label": "JSON", "value": "application/json"}
        ]
      }
    },
    {
      "key": "reject_type",
      "title": "拦截类型",
      "description": "拦截的文件类型(MIME)，支持 glob；命中即剔除",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true,
        "options": [
          {"label": "图片(image/*)", "value": "image/*"},
          {"label": "PDF", "value": "application/pdf"},
          {"label": "视频(video/*)", "value": "video/*"},
          {"label": "ZIP 压缩包", "value": "application/zip"}
        ]
      }
    },
    {
      "key": "pass_name",
      "title": "允许文件名",
      "description": "允许通过的文件名 glob，为空表示全部允许，如 *.png、report-*",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true
      }
    },
    {
      "key": "max_size",
      "title": "最大体积",
      "description": "单文件最大体积，支持带单位如 1kb/1mb/1g；为 0 或空表示不限",
      "config_type": "Custom",
      "default_value": "0",
      "form": {
        "type": "text"
      }
    }
  ]
}
```

- [ ] **Step 4: 提交**

```bash
git add file_uploader_plugins/resources/pre/upload_file_filter
git commit -m "refactor(plugins): rename file_type_filter resource to upload_file_filter, add name/size dims"
```

---

## Task 6: 重写 `upload_file_filter.rs`（三维度过滤 + 测试）

**Files:**
- Move: `file_uploader_plugins/src/pre_upload/file_type_filter.rs` → `file_uploader_plugins/src/pre_upload/upload_file_filter.rs`
- Modify: `file_uploader_plugins/src/pre_upload.rs`

- [ ] **Step 1: 重命名模块文件**

```bash
git mv file_uploader_plugins/src/pre_upload/file_type_filter.rs file_uploader_plugins/src/pre_upload/upload_file_filter.rs
```

- [ ] **Step 2: 更新 `pre_upload.rs` 模块声明**

把 `file_uploader_plugins/src/pre_upload.rs` 内容改为：

```rust
pub mod upload_file_filter;
```

- [ ] **Step 3: 重写 `upload_file_filter.rs`（完整内容）**

用以下内容整体替换 `file_uploader_plugins/src/pre_upload/upload_file_filter.rs`：

```rust
use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;
use glob::Pattern;
use std::sync::Arc;
use tracing::{info, warn};

pub struct UploadFileFilter;

/// 解析四项配置：pass_type / reject_type / pass_name（glob 字符串）+ max_size（字节）。
fn parse_config(
    config_info: &Arc<Option<serde_json::Value>>,
) -> (Vec<String>, Vec<String>, Vec<String>, Option<u64>) {
    (
        config_util::get_list(config_info, "pass_type"),
        config_util::get_list(config_info, "reject_type"),
        config_util::get_list(config_info, "pass_name"),
        config_util::get_size(config_info, "max_size"),
    )
}

/// 编译 glob 模式；无效模式记 warn 并跳过。
fn compile_patterns(items: &[String]) -> Vec<Pattern> {
    items
        .iter()
        .filter_map(|s| match Pattern::new(s) {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("upload_file_filter: invalid glob pattern '{}': {}", s, e);
                None
            }
        })
        .collect()
}

/// 三维度保留判定：
/// (pass_type 空 ∨ 命中) ∧ (未命中 reject_type) ∧ (pass_name 空 ∨ 命中) ∧ (size 不超限)
fn keep(
    f: &UploadFileData,
    pass_type: &[Pattern],
    reject_type: &[Pattern],
    pass_name: &[Pattern],
    max_size: Option<u64>,
) -> bool {
    let type_ok = pass_type.is_empty() || pass_type.iter().any(|p| p.matches(&f.file_type));
    let reject_ok = !reject_type.iter().any(|p| p.matches(&f.file_type));
    let name_ok = pass_name.is_empty() || pass_name.iter().any(|p| p.matches(&f.name));
    let size_ok = match max_size {
        Some(m) if m > 0 => (f.size as u64) <= m,
        _ => true,
    };
    type_ok && reject_ok && name_ok && size_ok
}

impl UploadPlugin for UploadFileFilter {
    fn name(&self) -> &'static str {
        "upload_file_filter"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::PreUpload
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        info!(
            "upload_file_filter input: {}",
            serde_json::to_string(ctx).unwrap_or_else(|_| "input error".to_string())
        );

        let (pass_type_raw, reject_type_raw, pass_name_raw, max_size) = parse_config(&ctx.config_info);
        let pass_type = compile_patterns(&pass_type_raw);
        let reject_type = compile_patterns(&reject_type_raw);
        let pass_name = compile_patterns(&pass_name_raw);

        let total = ctx.file_list.len();
        let filtered: Vec<_> = ctx
            .file_list
            .iter()
            .filter(|f| keep(f, &pass_type, &reject_type, &pass_name, max_size))
            .cloned()
            .collect();
        let passed = filtered.len();

        if filtered.is_empty() {
            return UploadOutputCtx::failed(format!(
                "upload_file_filter: all {} file(s) rejected (pass_type={:?}, reject_type={:?}, pass_name={:?}, max_size={:?})",
                total, pass_type_raw, reject_type_raw, pass_name_raw, max_size
            ));
        }

        UploadOutputCtx::success_files(
            format!("upload_file_filter: {}/{} passed", passed, total),
            filtered,
        )
    }

    fn on_load(&self) {
        info!("upload_file_filter: loading");
    }

    fn on_unload(&self) {
        info!("upload_file_filter: unloading");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::enums::FileDataType;
    use serde_json::Value;

    fn file(name: &str, file_type: &str, size: usize) -> Arc<UploadFileData> {
        Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            format!("/tmp/{name}"),
            name.to_string(),
            name.to_string(),
            file_type.to_string(),
            size,
        ))
    }

    fn run(files: Vec<Arc<UploadFileData>>, config: Option<Value>) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file_list: files,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir: None,
        };
        UploadFileFilter.execute(&ctx)
    }

    #[test]
    fn pass_type_keeps_only_matching() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn reject_type_drops_matching() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"reject_type": ["text/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn pass_name_glob_matches_suffix() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_name": ["*.png"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a.png");
    }

    #[test]
    fn pass_name_empty_passes_all() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_name": []});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn max_size_filters_oversized() {
        let files = vec![file("small", "image/png", 100), file("big", "image/png", 2_000_000)];
        let cfg = serde_json::json!({"max_size": "1mb"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "small");
    }

    #[test]
    fn max_size_zero_is_unlimited() {
        let files = vec![file("big", "image/png", 99_999_999)];
        let cfg = serde_json::json!({"max_size": "0"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn max_size_numeric_value_works() {
        let files = vec![file("a", "image/png", 50), file("b", "image/png", 200)];
        let cfg = serde_json::json!({"max_size": 100});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "a");
    }

    #[test]
    fn combine_all_three_dims() {
        let files = vec![
            file("ok.png", "image/png", 100),
            file("ok.txt", "text/plain", 100),
            file("big.png", "image/png", 9_000_000),
        ];
        // 类型 image/* ∧ 名称 *.png ∧ 大小 ≤1mb → 只剩 ok.png
        let cfg = serde_json::json!({"pass_type": ["image/*"], "pass_name": ["*.png"], "max_size": "1mb"});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
        assert_eq!(out.file_list.as_ref().unwrap()[0].name, "ok.png");
    }

    #[test]
    fn empty_config_passes_all() {
        let files = vec![file("a.png", "image/png", 1), file("b.txt", "text/plain", 1)];
        let out = run(files, None);
        assert_eq!(out.file_list.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn all_rejected_returns_failed() {
        let files = vec![file("a.txt", "text/plain", 1)];
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(files, Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
        assert!(out.file_list.is_none());
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        let files = vec![file("a.png", "image/png", 1)];
        let cfg = serde_json::json!({"pass_type": ["["]});
        let out = run(files, Some(cfg));
        assert_eq!(out.file_list.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let plugin = UploadFileFilter;
        plugin.on_load();
        plugin.on_unload();
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_plugins pre_upload::upload_file_filter`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```bash
git add file_uploader_plugins/src/pre_upload
git commit -m "feat(plugins): rewrite upload_file_filter with type/name/size dimensions"
```

---

## Task 7: 新增 `default_input_handler` 资源目录

**Files:**
- Create: `file_uploader_plugins/resources/input/default_input_handler/meta.json`
- Create: `file_uploader_plugins/resources/input/default_input_handler/config.json`

- [ ] **Step 1: 创建 `meta.json`**

写入 `file_uploader_plugins/resources/input/default_input_handler/meta.json`：

```json
{
  "name": "default_input_handler",
  "title": "默认输入处理器",
  "description": "将外部输入引入工作目录并嗅探文件类型：本地文件可选缓存，网络文件可选下载",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "Input"
}
```

- [ ] **Step 2: 创建 `config.json`**

写入 `file_uploader_plugins/resources/input/default_input_handler/config.json`：

```json
{
  "access": {
    "fs_read": true,
    "fs_write": true,
    "network": true
  },
  "params": [
    {
      "key": "cache_local",
      "title": "缓存本地文件",
      "description": "是否将本地文件拷贝一份到工作目录（true/false），默认 true",
      "config_type": "Default",
      "default_value": "true",
      "form": { "type": "text" }
    },
    {
      "key": "download_network",
      "title": "下载网络文件",
      "description": "是否默认下载网络文件到工作目录（true/false），默认 true；关闭则保留 URL 供下游自定义下载",
      "config_type": "Default",
      "default_value": "true",
      "form": { "type": "text" }
    },
    {
      "key": "sniff_type",
      "title": "嗅探文件类型",
      "description": "是否用魔数嗅探填充 file_type(MIME)（true/false），默认 true",
      "config_type": "Default",
      "default_value": "true",
      "form": { "type": "text" }
    }
  ]
}
```

- [ ] **Step 3: 提交**

```bash
git add file_uploader_plugins/resources/input
git commit -m "feat(plugins): add default_input_handler resource (meta + config)"
```

---

## Task 8: 实现 `default_input_handler.rs`（+ 测试）

**Files:**
- Create: `file_uploader_plugins/src/input.rs`
- Create: `file_uploader_plugins/src/input/default_input_handler.rs`
- Modify: `file_uploader_plugins/src/lib.rs`

- [ ] **Step 1: 创建 `input.rs` 模块入口**

写入 `file_uploader_plugins/src/input.rs`：

```rust
pub mod default_input_handler;
```

- [ ] **Step 2: 在 `lib.rs` 注册 input 模块**

把 `file_uploader_plugins/src/lib.rs` 改为：

```rust
pub mod input;
pub mod post_upload;
pub mod pre_upload;
pub mod upload;
```

- [ ] **Step 3: 写 `default_input_handler.rs`（完整实现 + 测试）**

写入 `file_uploader_plugins/src/input/default_input_handler.rs`：

```rust
use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{FileDataType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::{config_util, fs_util};
use file_uploader_sdk::error::UploadError;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

pub struct DefaultInputHandler;

impl UploadPlugin for DefaultInputHandler {
    fn name(&self) -> &'static str {
        "default_input_handler"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::Input
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        match self.run(ctx) {
            Ok(files) => UploadOutputCtx::success_files(
                format!("default_input_handler: processed {} file(s)", files.len()),
                files,
            ),
            Err(e) => UploadOutputCtx::failed(format!("default_input_handler: {e}")),
        }
    }

    fn on_load(&self) {
        info!("default_input_handler: loading");
    }

    fn on_unload(&self) {
        info!("default_input_handler: unloading");
    }
}

impl DefaultInputHandler {
    fn run(&self, ctx: &UploadInputCtx) -> Result<Vec<Arc<UploadFileData>>, UploadError> {
        let cache_local = config_util::get_bool(&ctx.config_info, "cache_local").unwrap_or(true);
        let download_network =
            config_util::get_bool(&ctx.config_info, "download_network").unwrap_or(true);
        let sniff_type = config_util::get_bool(&ctx.config_info, "sniff_type").unwrap_or(true);
        let work_dir = ctx.work_dir.as_deref().unwrap_or("");

        let mut out = Vec::with_capacity(ctx.file_list.len());
        for f in &ctx.file_list {
            let mut nf = (**f).clone();
            match nf.data_type {
                FileDataType::FilePath => {
                    if sniff_type {
                        if let Some(mime) = sniff_external(&nf.input_path) {
                            nf.file_type = mime;
                        } else {
                            warn!("default_input_handler: sniff failed for {}", nf.input_path);
                        }
                    }
                    if cache_local {
                        let wd = require_work_dir(work_dir)?;
                        let ext = ext_from_name(&nf.name);
                        let (name, path) = fs_util::import_file(wd, &nf.input_path, ext)?;
                        nf.size = fs_util::file_size(wd, &name)? as usize;
                        nf.input_path = path.to_string_lossy().into_owned();
                    }
                }
                FileDataType::NetworkPath => {
                    if download_network {
                        let wd = require_work_dir(work_dir)?;
                        let ext = ext_from_name(&nf.name);
                        let (name, path) = download_to_workdir(wd, &nf.input_path, ext)?;
                        nf.size = fs_util::file_size(wd, &name)? as usize;
                        nf.input_path = path.to_string_lossy().into_owned();
                        if sniff_type {
                            if let Some(mime) = sniff_in_workdir(wd, &name) {
                                nf.file_type = mime;
                            } else {
                                warn!("default_input_handler: sniff failed for {}", name);
                            }
                        }
                    }
                    // download_network=false → 保留 URL，不嗅探（透传）
                }
                FileDataType::Binary => {
                    // 原样透传（字段预留，当前不处理内存数据）
                }
            }
            out.push(Arc::new(nf));
        }
        Ok(out)
    }
}

fn require_work_dir(work_dir: &str) -> Result<&str, UploadError> {
    if work_dir.is_empty() {
        Err(UploadError::WorkDirNotSet)
    } else {
        Ok(work_dir)
    }
}

fn ext_from_name(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && !ext.contains('/') => ext,
        _ => "",
    }
}

fn sniff_bytes(bytes: &[u8]) -> Option<String> {
    infer::get(bytes).map(|t| t.mime_type().to_string())
}

fn sniff_external(src_abs_path: &str) -> Option<String> {
    let head = fs_util::read_external_head(src_abs_path, 512).ok()?;
    sniff_bytes(&head)
}

fn sniff_in_workdir(work_dir: &str, filename: &str) -> Option<String> {
    let mut r = fs_util::open_read(work_dir, filename).ok()?;
    let mut buf = vec![0u8; 512];
    let n = r.read(&mut buf).ok()?;
    buf.truncate(n);
    sniff_bytes(&buf)
}

fn download_to_workdir(
    work_dir: &str,
    url: &str,
    ext: &str,
) -> Result<(String, PathBuf), UploadError> {
    let resp = reqwest::blocking::get(url).map_err(|e| {
        UploadError::PluginLoadError(format!("download failed for {url}: {e}"))
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(UploadError::PluginLoadError(format!(
            "download {url} returned status {status}"
        )));
    }
    fs_util::write(work_dir, ext, resp)
}

写入文件的完整内容如下（实现 + helper + 一份测试模块）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::enums::OutputResultType;
    use file_uploader_sdk::utils::fs_util::gen_unique_name;

    fn tmp_work_dir() -> String {
        let dir = std::env::temp_dir().join(format!("dih_{}", gen_unique_name("")));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn write_png(path: &str) {
        // 最小 PNG 魔数头（infer 识别为 image/png）
        let png = [
            0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
            0x49, 0x48, 0x44, 0x52,
        ];
        std::fs::write(path, png).unwrap();
    }

    fn run(
        files: Vec<Arc<UploadFileData>>,
        config: Option<serde_json::Value>,
        work_dir: Option<String>,
    ) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file_list: files,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir,
        };
        DefaultInputHandler.execute(&ctx)
    }

    #[test]
    fn local_file_cached_and_sniffed() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath, src, "id1".into(), "src.png".into(), String::new(), 0,
        ));
        let cfg = serde_json::json!({"cache_local": true, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd.clone()));
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_type, "image/png");
        // input_path 应已改为 work_dir 内副本
        assert!(list[0].input_path.starts_with(&wd));
        assert!(list[0].input_path.ends_with(".png"));
        assert!(list[0].size > 0);
    }

    #[test]
    fn local_file_no_cache_keeps_path_but_sniffs() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath, src.clone(), "id1".into(), "src.png".into(), String::new(), 0,
        ));
        let cfg = serde_json::json!({"cache_local": false, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd));
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        // 未缓存：input_path 保留原值，但 file_type 已嗅探
        assert_eq!(list[0].input_path, src);
        assert_eq!(list[0].file_type, "image/png");
    }

    #[test]
    fn sniff_disabled_keeps_original_type() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath, src, "id1".into(), "src.png".into(), "upstream/x".into(), 0,
        ));
        let cfg = serde_json::json!({"cache_local": false, "sniff_type": false});
        let out = run(vec![f], Some(cfg), Some(wd));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].file_type, "upstream/x");
    }

    #[test]
    fn binary_file_pass_through() {
        let f = Arc::new(UploadFileData::new(
            FileDataType::Binary, String::new(), "id1".into(), "blob".into(), "x/y".into(), 7,
        ));
        let out = run(vec![f], None, None);
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].name, "blob");
        assert_eq!(list[0].file_type, "x/y"); // 原样未改
    }

    #[test]
    fn network_no_download_passes_through() {
        let f = Arc::new(UploadFileData::new(
            FileDataType::NetworkPath, "https://example.com/a.png".into(), "id1".into(),
            "a.png".into(), "upstream/png".into(), 0,
        ));
        let cfg = serde_json::json!({"download_network": false});
        let out = run(vec![f], Some(cfg), None);
        assert!(matches!(out.result, OutputResultType::Success));
        let list = out.file_list.as_ref().unwrap();
        assert_eq!(list[0].input_path, "https://example.com/a.png"); // URL 保留
        assert_eq!(list[0].file_type, "upstream/png"); // 未嗅探
    }

    #[test]
    fn missing_work_dir_when_cache_required_fails() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath, src, "id1".into(), "src.png".into(), String::new(), 0,
        ));
        // cache_local=true 但 work_dir=None → Failed
        let cfg = serde_json::json!({"cache_local": true});
        let out = run(vec![f], Some(cfg), None);
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn local_missing_source_fails() {
        let wd = tmp_work_dir();
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath, format!("{wd}/nope.png"), "id1".into(), "nope.png".into(),
            String::new(), 0,
        ));
        let cfg = serde_json::json!({"cache_local": true});
        let out = run(vec![f], Some(cfg), Some(wd));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    #[ignore = "needs network; run with: cargo test -p file_uploader_plugins -- --ignored download_network"]
    fn download_network_real() {
        let wd = tmp_work_dir();
        // 使用一个稳定的、返回 PNG 字节的小型公共资源；若无则跳过。
        // 占位 URL：执行者可替换为内网可用的测试地址。
        let url = "https://www.w3.org/Icons/w3c_main.png";
        let f = Arc::new(UploadFileData::new(
            FileDataType::NetworkPath, url.into(), "id1".into(), "logo.png".into(), String::new(), 0,
        ));
        let cfg = serde_json::json!({"download_network": true, "sniff_type": true});
        let out = run(vec![f], Some(cfg), Some(wd.clone()));
        if matches!(out.result, OutputResultType::Failed) {
            eprintln!("network test failed (expected offline): {}", out.message);
            return;
        }
        let list = out.file_list.as_ref().unwrap();
        assert!(list[0].input_path.starts_with(&wd));
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let p = DefaultInputHandler;
        p.on_load();
        p.on_unload();
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_plugins input::default_input_handler`
Expected: 非 ignored 测试全部 PASS（`download_network_real` 显示 ignored）

- [ ] **Step 5: 手动验证网络下载（可选）**

Run: `cargo test -p file_uploader_plugins -- --ignored download_network`
Expected: 通过（离线时输出 "network test failed (expected offline)" 并通过）

- [ ] **Step 6: 提交**

```bash
git add file_uploader_plugins/src/input file_uploader_plugins/src/lib.rs
git commit -m "feat(plugins): add default_input_handler (cache/download/sniff) for Input phase"
```

---

## Task 9: 更新 `file_uploader_core/src/main.rs` 注册示例

**Files:**
- Modify: `file_uploader_core/src/main.rs:6,33,36`

- [ ] **Step 1: 更新 import**

把 `main.rs` 第 6 行：

```rust
use file_uploader_plugins::pre_upload::file_type_filter::FileTypeFilter;
```

改为：

```rust
use file_uploader_plugins::input::default_input_handler::DefaultInputHandler;
use file_uploader_plugins::pre_upload::upload_file_filter::UploadFileFilter;
```

- [ ] **Step 2: 更新资源路径与插件实例**

把 `main.rs` 第 32-40 行（`=== Testing IN-PROCESS plugin ===` 段）：

```rust
    info!("=== Testing IN-PROCESS plugin ===");
    let in_process_dir = target_dir.join("resources/pre/file_type_filter");
    let Ok(plugin) = UploadPluginInfo::new_in_process(
        in_process_dir.to_str().unwrap(),
        Box::new(FileTypeFilter),
    ) else {
        error!("Plugin load error");
        return;
    };
```

改为（同时注册 input 与 pre 两个进程内插件）：

```rust
    info!("=== Testing IN-PROCESS plugins ===");
    let input_dir = target_dir.join("resources/input/default_input_handler");
    let Ok(input_plugin) = UploadPluginInfo::new_in_process(
        input_dir.to_str().unwrap(),
        Box::new(DefaultInputHandler),
    ) else {
        error!("Input plugin load error");
        return;
    };
    info!("Input plugin loaded: {}", input_plugin.id);

    let filter_dir = target_dir.join("resources/pre/upload_file_filter");
    let Ok(filter_plugin) = UploadPluginInfo::new_in_process(
        filter_dir.to_str().unwrap(),
        Box::new(UploadFileFilter),
    ) else {
        error!("Filter plugin load error");
        return;
    };
    info!("Filter plugin loaded: {}", filter_plugin.id);

    let in_process_dir = filter_dir.clone();
    let plugin = filter_plugin;
```

> 保留后续 `plugin.slot.execute(&ctx)` 调用不变（仍演示一次执行）。

- [ ] **Step 3: 构建验证**

Run: `cargo build -p file_uploader_core`
Expected: 编译通过

- [ ] **Step 4: 提交**

```bash
git add file_uploader_core/src/main.rs
git commit -m "chore(core): update main.rs to register upload_file_filter + default_input_handler"
```

---

## Task 10: 全量构建与测试验证

**Files:** 无（仅验证）

- [ ] **Step 1: 清理旧的 target 资源残留（避免旧目录干扰）**

```bash
rm -rf target/debug/resources target/release/resources
```

> `build.rs` 是覆盖式复制，不会删除已改名/移除的旧目录；手动清理确保 `target/resources/pre/file_type_filter` 旧副本不残留。

- [ ] **Step 2: 全量构建**

Run: `cargo build`
Expected: 编译通过（重新触发 build.rs 复制新资源树）

- [ ] **Step 3: 全量测试**

Run: `cargo test`
Expected: 全部 PASS（`download_network_real` ignored）

- [ ] **Step 4: 运行核心示例确认资源加载**

Run: `cargo run -p file_uploader_core`
Expected: 输出 `Input plugin loaded: in_process_Input_default_input_handler` 与 `Filter plugin loaded: in_process_PreUpload_upload_file_filter`，执行无 panic

- [ ] **Step 5: 构建 dylib 插件确认 ABI 未受影响**

Run: `cargo build -p uploader_example_plugin`
Expected: 编译通过（stabby ABI 未变）

- [ ] **Step 6: 最终提交（如有改动）**

```bash
git add -A
git commit -m "test: verify full build and test suite for plugin iteration" --allow-empty
```

---

## 自审备注（plan self-review）

**Spec 覆盖核对：**
- §2 决策表逐项 → Task 1/2(parse_size,get_size,import_file)、Task 5/6(三维度)、Task 7/8(input 插件)、Task 3(Clone+注释) ✓
- §4 三维度过滤 → Task 6 ✓
- §5 分流 + 嗅探 + 容错 → Task 8 ✓
- §6 SDK 改动 → Task 1/2/3 ✓
- §7 依赖 → Task 4 ✓
- §10 文件清单 → 各 Task 对应 ✓

**类型一致性：**
- `parse_size(s: &str) -> Option<u64>`、`get_size(config, key) -> Option<u64>`（Task 1 定义，Task 6 调用）✓
- `import_file(work_dir, src, ext) -> Result<(String, PathBuf)>`、`read_external_head(src, n) -> Result<Vec<u8>>`、`file_size(work_dir, filename) -> Result<u64>`（Task 2 定义，Task 8 调用）✓
- `UploadFileFilter`（Task 6）/ `DefaultInputHandler`（Task 8）结构名与 main.rs（Task 9）一致 ✓

**已知风险：**
- Task 3 依赖 stabby `SArc`/`SVec` 支持 `Clone`；若不支持需 fallback 到手动重建（已注明）
- Task 8 网络测试默认 `#[ignore]`，CI 不依赖真实网络
