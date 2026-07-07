# UploadInputCtx 单文件化重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `UploadInputCtx` / `UploadOutputCtx` 的文件承载字段从 `Vec<Arc<UploadFileData>>` 收敛为 `Option<Arc<UploadFileData>>`，同步改造 stabby 层、转换层、pipeline 流转、内置插件、示例插件、测试与文档。

**Architecture:** 自底向上按 crate 切片——先 SDK（被依赖的底层），再 plugins（仅依赖 SDK），最后 core + example（依赖前两者）。每个 task 结束时对应 crate 能独立 `cargo build` + `cargo test` 通过。`upload_file_filter` 同步重命名为 `upload_file_validator`（批量过滤→单文件校验）。

**Tech Stack:** Rust edition 2024、stabby（ABI 稳定层）、serde、tracing、thiserror、glob。测试用 `cargo test`（含 `#[cfg(test)] mod tests`）。

---

## 文件结构

| 文件 | 责任 | 本次变更 |
|---|---|---|
| `file_uploader_sdk/src/models/ctx.rs` | 原生上下文结构 + `UploadOutputCtx` 关联函数 | 改 `UploadInputCtx.file` / `UploadOutputCtx.file` / `success_file` / 测试 |
| `file_uploader_sdk/src/models/ctx_stabby.rs` | stabby ABI 结构 | 改 `UploadInputCtxS.file` / `UploadOutputCtxS.file` |
| `file_uploader_sdk/src/utils/ctx_util.rs` | 原生⇄stabby 双向转换 | 改 `convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx` / 测试 |
| `file_uploader_plugins/src/input/default_input_handler.rs` | Input 阶段内置插件 | 去循环、改返回类型、改输出、改测试 |
| `file_uploader_plugins/src/pre_upload/upload_file_filter.rs` → `upload_file_validator.rs` | PreUpload 阶段插件 | 重命名 + 语义变更（filter→validator）+ 改测试 |
| `file_uploader_plugins/src/pre_upload.rs` | PreUpload 模块声明 | `pub mod` 改名 |
| `file_uploader_plugins/resources/pre/upload_file_filter/` → `upload_file_validator/` | 插件资源 | 目录迁移 + meta.json `name` |
| `file_uploader_core/src/pipeline/registry.rs` | pipeline 执行 + 注册表 | 改 `output_to_input` / `execute_pipeline` / 测试构造点 |
| `file_uploader_core/src/main.rs` | 示例入口 | 改字段 + filter 引用改名 |
| `uploader_example_plugin/src/lib.rs` | 示例 dylib 插件 | 改字段 |
| `README.md` / `AGENTS.md` / `.agents/skills/designing-in-process-plugins/SKILL.md` | 文档 | 字段名 + 示例同步 |

---

## Task 1: SDK 层单文件化（ctx + stabby + util + 测试）

**Files:**
- Modify: `file_uploader_sdk/src/models/ctx.rs`
- Modify: `file_uploader_sdk/src/models/ctx_stabby.rs`
- Modify: `file_uploader_sdk/src/utils/ctx_util.rs`

- [ ] **Step 1: 改 `ctx.rs` 的 `UploadInputCtx` 字段**

Modify `file_uploader_sdk/src/models/ctx.rs:178-192`，将 `file_list` 改为 `file`：

```rust
#[derive(Serialize, Deserialize)]
pub struct UploadInputCtx {
    // 文件数据
    pub file: Option<Arc<UploadFileData>>,
    // 配置信息
    pub config_info: Arc<Option<Value>>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
    // 关联流程
    #[serde(skip)]
    pub related_process_info: Option<Weak<UploadProcessCtx>>,
    // 活动目录：本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传）。
    // None 表示未设置。#[serde(default)] 兼容旧 JSON。
    #[serde(default)]
    pub work_dir: Option<String>,
}
```

- [ ] **Step 2: 改 `ctx.rs` 的 `UploadOutputCtx` 字段**

Modify `file_uploader_sdk/src/models/ctx.rs:197-207`：

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct UploadOutputCtx {
    // 输出结果
    pub result: OutputResultType,
    // 输出信息
    pub message: String,
    // 文件数据
    pub file: Option<Arc<UploadFileData>>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
}
```

- [ ] **Step 3: 改 `ctx.rs` 的关联函数（`success_files` → `success_file`）**

Modify `file_uploader_sdk/src/models/ctx.rs:209-249`，将 `success_files` 替换为 `success_file`，其余 `success`/`failed`/`interrupt` 内部 `file_list: None` → `file: None`：

```rust
impl UploadOutputCtx {
    /// 成功（无文件产出）
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }

    /// 成功（携带处理后文件）
    pub fn success_file(msg: impl Into<String>, file: Arc<UploadFileData>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file: Some(file),
            extra_info: None,
        }
    }

    /// 失败（会中断 pipeline）
    pub fn failed(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Failed,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }

    /// 中断
    pub fn interrupt(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Interrupt,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }
}
```

- [ ] **Step 4: 改 `ctx.rs` 的 `output_helper_tests`**

Modify `file_uploader_sdk/src/models/ctx.rs:251-294`（测试模块），将 `success_files`/`file_list` 适配为单文件：

```rust
#[cfg(test)]
mod output_helper_tests {
    use super::*;
    use crate::models::enums::OutputResultType;

    #[test]
    fn success_has_no_file() {
        let o = UploadOutputCtx::success("ok");
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.message, "ok");
        assert!(o.file.is_none());
        assert!(o.extra_info.is_none());
    }

    #[test]
    fn success_file_carries_file() {
        let f = Arc::new(UploadFileData::new(
            crate::models::enums::FileDataType::FilePath,
            "/tmp/a".into(),
            "a".into(),
            "a".into(),
            "image/png".into(),
            0,
        ));
        let o = UploadOutputCtx::success_file("done", f);
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.file.as_ref().unwrap().name, "a");
    }

    #[test]
    fn failed_sets_failed_result() {
        let o = UploadOutputCtx::failed("boom");
        assert!(matches!(o.result, OutputResultType::Failed));
        assert_eq!(o.message, "boom");
        assert!(o.file.is_none());
    }

    #[test]
    fn interrupt_sets_interrupt_result() {
        let o = UploadOutputCtx::interrupt("stop");
        assert!(matches!(o.result, OutputResultType::Interrupt));
        assert_eq!(o.message, "stop");
    }
}
```

- [ ] **Step 5: 改 `ctx_stabby.rs` 的 stabby 结构**

Modify `file_uploader_sdk/src/models/ctx_stabby.rs:28-56`，`file_list` → `file`（单文件）：

```rust
/**
 * 输入上下文
 */
#[stabby::stabby]
pub struct UploadInputCtxS {
    // 文件数据
    pub file: SOption<SArc<UploadFileDataS>>,
    // 配置信息
    pub config_info: SOption<SString>,
    // 扩展信息
    pub extra_info: SOption<SString>,
    // 活动目录
    pub work_dir: SOption<SString>,
}

/**
 * 输出结果
 */
#[stabby::stabby]
pub struct UploadOutputCtxS {
    // 输出结果
    pub result: OutputResultType,
    // 输出信息
    pub message: SString,
    // 文件数据
    pub file: SOption<SArc<UploadFileDataS>>,
    // 扩展信息
    pub extra_info: SOption<SString>,
}
```

`UploadFileDataS`（`:7-26`）不变。

- [ ] **Step 6: 改 `ctx_util.rs` 的 `convert_input_ctx_s`**

Modify `file_uploader_sdk/src/utils/ctx_util.rs:24-64`，将 `file_list` 的 `iter().map().collect()` 替换为单文件转换：

```rust
pub fn convert_input_ctx_s(input: &UploadInputCtx) -> UploadInputCtxS {
    let file = match &input.file {
        Some(f) => SOption::Some(SArc::new(convert_file_data_s(f))),
        None => stabby::option::Option::None(),
    };

    let extra_info: SOption<SString> = match &input.extra_info {
        None => None.into(),
        Some(map) => {
            if let Ok(json) = serde_json::to_string(map) {
                SOption::Some(json.into())
            } else {
                error!("Failed to serialize extra info");
                None.into()
            }
        }
    };

    let config_info: SOption<SString> = match &*input.config_info {
        None => None.into(),
        Some(config) => {
            if let Ok(json) = serde_json::to_string(config) {
                SOption::Some(json.into())
            } else {
                error!("Failed to serialize config info");
                None.into()
            }
        }
    };

    UploadInputCtxS {
        file,
        config_info,
        extra_info,
        work_dir: match &input.work_dir {
            None => None.into(),
            Some(s) => SOption::Some(s.clone().into()),
        },
    }
}
```

- [ ] **Step 7: 改 `ctx_util.rs` 的 `convert_input_ctx`**

Modify `file_uploader_sdk/src/utils/ctx_util.rs:66-94`：

```rust
pub fn convert_input_ctx(input: &UploadInputCtxS) -> UploadInputCtx {
    let file: Option<Arc<UploadFileData>> = input.file.match_ref(
        |f| Some(Arc::new(convert_file_data(f))),
        || None,
    );

    let extra_info: Option<HashMap<String, String>> = input.extra_info.match_ref(
        |extra_info_s| {
            return if let Ok(map) = serde_json::from_str(extra_info_s) {
                Some(map)
            } else {
                error!("Failed to deserialize extra info");
                None
            };
        },
        || None,
    );
    UploadInputCtx {
        file,
        config_info: input.config_info.match_ref(
            |config_info_s| Arc::new(get_config(config_info_s)),
            || Arc::new(None),
        ),
        extra_info,
        related_process_info: None,
        work_dir: input.work_dir.match_ref(|s| Some(s.clone().into()), || None),
    }
}
```

- [ ] **Step 8: 改 `ctx_util.rs` 的 `convert_output_ctx`**

Modify `file_uploader_sdk/src/utils/ctx_util.rs:108-138`：

```rust
pub fn convert_output_ctx(input: &UploadOutputCtxS) -> UploadOutputCtx {
    let file: Option<Arc<UploadFileData>> = input.file.match_ref(
        |f| Some(Arc::new(convert_file_data(f))),
        || None,
    );

    let extra_info: Option<HashMap<String, String>> = input.extra_info.match_ref(
        |extra_info_s| {
            return if let Ok(map) = serde_json::from_str(extra_info_s) {
                Some(map)
            } else {
                error!("Failed to deserialize extra info");
                None
            };
        },
        || None,
    );

    UploadOutputCtx {
        result: input.result.clone(),
        message: input.message.clone().into(),
        file,
        extra_info,
    }
}
```

- [ ] **Step 9: 改 `ctx_util.rs` 的 `work_dir_tests`**

Modify `file_uploader_sdk/src/utils/ctx_util.rs:140-178`（测试模块），构造点 `file_list: vec![...]` → `file: Some(Arc::new(...))`：

```rust
#[cfg(test)]
mod work_dir_tests {
    use super::*;
    use crate::models::ctx::UploadFileData;
    use crate::models::enums::FileDataType;

    fn input_with_work_dir(wd: Option<&str>) -> UploadInputCtx {
        UploadInputCtx {
            file: Some(Arc::new(UploadFileData::new(
                FileDataType::FilePath,
                "/tmp/a".into(),
                "a".into(),
                "a".into(),
                "image/png".into(),
                0,
            ))),
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

`get_config`（`:180-188`）不变。

- [ ] **Step 10: 验证 SDK crate 编译与测试**

Run: `cargo build -p file_uploader_sdk`
Expected: 编译通过（SDK 自身无外部依赖引用旧字段）。

Run: `cargo test -p file_uploader_sdk`
Expected: `output_helper_tests::*` 与 `work_dir_tests::*` 全部 PASS。

> 注意：此时 `file_uploader_core` / `file_uploader_plugins` / `uploader_example_plugin` 因仍引用 `file_list` 会编译失败，属预期，后续 Task 修复。

- [ ] **Step 11: Commit**

```bash
git add file_uploader_sdk/src/models/ctx.rs file_uploader_sdk/src/models/ctx_stabby.rs file_uploader_sdk/src/utils/ctx_util.rs
git commit -m "refactor(sdk): UploadInputCtx/UploadOutputCtx 单文件化

file_list: Vec<Arc<UploadFileData>> → file: Option<Arc<UploadFileData>>
- ctx.rs: 字段 + success_files→success_file + 测试
- ctx_stabby.rs: UploadInputCtxS/UploadOutputCtxS 同步
- ctx_util.rs: 双向转换去 Vec 重建 + 测试"
```

---

## Task 2: plugins crate 适配（input_handler + filter→validator + 资源 + 模块）

**Files:**
- Modify: `file_uploader_plugins/src/input/default_input_handler.rs`
- Rename: `file_uploader_plugins/src/pre_upload/upload_file_filter.rs` → `file_uploader_plugins/src/pre_upload/upload_file_validator.rs`
- Modify: `file_uploader_plugins/src/pre_upload.rs`
- Rename: `file_uploader_plugins/resources/pre/upload_file_filter/` → `file_uploader_plugins/resources/pre/upload_file_validator/`
- Modify: `file_uploader_plugins/resources/pre/upload_file_validator/meta.json`

- [ ] **Step 1: 改 `default_input_handler.rs` 的 `execute` 与 `run`**

Modify `file_uploader_plugins/src/input/default_input_handler.rs:22-93`。

`execute`（`:22-30`）改为根据 `run` 返回的 `Option` 选择 `success_file` 或 `success`：

```rust
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        match self.run(ctx) {
            Ok(Some(file)) => UploadOutputCtx::success_file(
                format!("default_input_handler: processed 1 file"),
                file,
            ),
            Ok(None) => UploadOutputCtx::success(
                "default_input_handler: no file to process",
            ),
            Err(e) => UploadOutputCtx::failed(format!("default_input_handler: {e}")),
        }
    }
```

`run`（`:42-92`）返回类型改为 `Result<Option<Arc<UploadFileData>>, UploadError>`，去掉循环：

```rust
    fn run(&self, ctx: &UploadInputCtx) -> Result<Option<Arc<UploadFileData>>, UploadError> {
        let cache_local = config_util::get_bool(&ctx.config_info, "cache_local").unwrap_or(true);
        let download_network =
            config_util::get_bool(&ctx.config_info, "download_network").unwrap_or(true);
        let sniff_type = config_util::get_bool(&ctx.config_info, "sniff_type").unwrap_or(true);
        let work_dir = ctx.work_dir.as_deref().unwrap_or("");

        let Some(f) = ctx.file.as_ref() else {
            return Ok(None);
        };
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
            }
            FileDataType::Binary => {
                // 原样透传（字段预留，当前不处理内存数据）
            }
        }
        Ok(Some(Arc::new(nf)))
    }
```

`require_work_dir` / `ext_from_name` / `sniff_*` / `download_to_workdir`（`:95-141`）不变。

- [ ] **Step 2: 改 `default_input_handler.rs` 的测试**

Modify `file_uploader_plugins/src/input/default_input_handler.rs:143-341`（测试模块）。`run` 辅助函数与断言适配单文件：

`run` 辅助函数（`:164-177`）改为：

```rust
    fn run(
        file: Option<Arc<UploadFileData>>,
        config: Option<serde_json::Value>,
        work_dir: Option<String>,
    ) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir,
        };
        DefaultInputHandler.execute(&ctx)
    }
```

各测试用例的调用从 `run(vec![f], cfg, wd)` → `run(Some(f), cfg, wd)`；断言从 `out.file_list.as_ref().unwrap()` / `[0].xxx` → `out.file.as_ref().unwrap().xxx`。

示例改法（`local_file_cached_and_sniffed`，`:179-201`）：

```rust
    #[test]
    fn local_file_cached_and_sniffed() {
        let wd = tmp_work_dir();
        let src = format!("{wd}/src.png");
        write_png(&src);
        let f = Arc::new(UploadFileData::new(
            FileDataType::FilePath,
            src,
            "id1".into(),
            "src.png".into(),
            String::new(),
            0,
        ));
        let cfg = serde_json::json!({"cache_local": true, "sniff_type": true});
        let out = run(Some(f), Some(cfg), Some(wd.clone()));
        assert!(matches!(out.result, OutputResultType::Success));
        let got = out.file.as_ref().unwrap();
        assert_eq!(got.file_type, "image/png");
        assert!(got.input_path.starts_with(&wd));
        assert!(got.input_path.ends_with(".png"));
        assert!(got.size > 0);
    }
```

对其余用例（`local_file_no_cache_keeps_path_but_sniffs` / `sniff_disabled_keeps_original_type` / `binary_file_pass_through` / `network_no_download_passes_through` / `missing_work_dir_when_cache_required_fails` / `local_missing_source_fails` / `download_network_real`）应用同一机械变换：`vec![f]` → `Some(f)`，`out.file_list.as_ref().unwrap()` → `out.file.as_ref().unwrap()`，`[0].xxx` → `.xxx`。`on_load_and_unload_do_not_panic` 不涉及文件，不改。

- [ ] **Step 3: 重命名 filter 源文件**

Run:
```bash
git mv file_uploader_plugins/src/pre_upload/upload_file_filter.rs file_uploader_plugins/src/pre_upload/upload_file_validator.rs
```

- [ ] **Step 4: 改写 `upload_file_validator.rs` 的 struct 与 name**

Modify `file_uploader_plugins/src/pre_upload/upload_file_validator.rs:9` 与 `:56-64`。

struct 与 `name()` 改名：

```rust
pub struct UploadFileValidator;
```

```rust
impl UploadPlugin for UploadFileValidator {
    fn name(&self) -> &'static str {
        "upload_file_validator"
    }

    fn phase(&self) -> UploadPhase {
        UploadPhase::PreUpload
    }
```

`parse_config`（`:12-21`）/ `compile_patterns`（`:24-35`）/ `keep`（`:39-54`）三个辅助函数**保留不变**。

- [ ] **Step 5: 改写 `upload_file_validator.rs` 的 `execute`（语义变更：filter→单文件校验）**

Modify `file_uploader_plugins/src/pre_upload/upload_file_validator.rs:65-96`。原批量 filter 替换为单文件 accept/reject：

```rust
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        info!(
            "upload_file_validator input: {}",
            serde_json::to_string(ctx).unwrap_or_else(|_| "input error".to_string())
        );

        let Some(f) = ctx.file.as_ref() else {
            return UploadOutputCtx::failed("upload_file_validator: no file to validate");
        };

        let (pass_type_raw, reject_type_raw, pass_name_raw, max_size) = parse_config(&ctx.config_info);
        let pass_type = compile_patterns(&pass_type_raw);
        let reject_type = compile_patterns(&reject_type_raw);
        let pass_name = compile_patterns(&pass_name_raw);

        if keep(f, &pass_type, &reject_type, &pass_name, max_size) {
            UploadOutputCtx::success_file(
                "upload_file_validator: accepted".to_string(),
                f.clone(),
            )
        } else {
            UploadOutputCtx::failed(format!(
                "upload_file_validator: rejected (pass_type={:?}, reject_type={:?}, pass_name={:?}, max_size={:?})",
                pass_type_raw, reject_type_raw, pass_name_raw, max_size
            ))
        }
    }
```

`on_load` / `on_unload`（`:98-104`）保留，仅将日志里的 `upload_file_filter` 改为 `upload_file_validator`：

```rust
    fn on_load(&self) {
        info!("upload_file_validator: loading");
    }

    fn on_unload(&self) {
        info!("upload_file_validator: unloading");
    }
```

- [ ] **Step 6: 改写 `upload_file_validator.rs` 的测试（删多文件用例，改单文件用例）**

Modify `file_uploader_plugins/src/pre_upload/upload_file_validator.rs:107-241`（整个测试模块）。`file` 辅助保留；`run` 改为单文件签名；删除断言 `len==2` 的多文件用例（`pass_name_empty_passes_all` / `empty_config_passes_all`），改写其余为单文件 accept/reject。

替换整个测试模块为：

```rust
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

    fn run(f: Option<Arc<UploadFileData>>, config: Option<Value>) -> UploadOutputCtx {
        let ctx = UploadInputCtx {
            file: f,
            config_info: Arc::new(config),
            extra_info: None,
            related_process_info: None,
            work_dir: None,
        };
        UploadFileValidator.execute(&ctx)
    }

    #[test]
    fn pass_type_accepts_matching() {
        let f = file("a.png", "image/png", 1);
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file.as_ref().unwrap().name, "a.png");
    }

    #[test]
    fn pass_type_rejects_non_matching() {
        let f = file("a.txt", "text/plain", 1);
        let cfg = serde_json::json!({"pass_type": ["image/*"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
        assert!(out.file.is_none());
    }

    #[test]
    fn reject_type_drops_matching() {
        let f = file("a.txt", "text/plain", 1);
        let cfg = serde_json::json!({"reject_type": ["text/*"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn reject_type_keeps_non_matching() {
        let f = file("a.png", "image/png", 1);
        let cfg = serde_json::json!({"reject_type": ["text/*"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
        assert_eq!(out.file.as_ref().unwrap().name, "a.png");
    }

    #[test]
    fn pass_name_glob_accepts_suffix() {
        let f = file("a.png", "image/png", 1);
        let cfg = serde_json::json!({"pass_name": ["*.png"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn pass_name_glob_rejects_non_suffix() {
        let f = file("a.txt", "text/plain", 1);
        let cfg = serde_json::json!({"pass_name": ["*.png"]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn max_size_accepts_under_limit() {
        let f = file("small", "image/png", 100);
        let cfg = serde_json::json!({"max_size": "1mb"});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn max_size_rejects_over_limit() {
        let f = file("big", "image/png", 2_000_000);
        let cfg = serde_json::json!({"max_size": "1mb"});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn max_size_zero_is_unlimited() {
        let f = file("big", "image/png", 99_999_999);
        let cfg = serde_json::json!({"max_size": "0"});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn max_size_numeric_value_works() {
        let f = file("a", "image/png", 50);
        let cfg = serde_json::json!({"max_size": 100});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn max_size_numeric_rejects_over() {
        let f = file("b", "image/png", 200);
        let cfg = serde_json::json!({"max_size": 100});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn empty_config_accepts() {
        let f = file("a.png", "image/png", 1);
        let out = run(Some(f), None);
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn invalid_pattern_is_skipped() {
        let f = file("a.png", "image/png", 1);
        let cfg = serde_json::json!({"pass_type": ["["]});
        let out = run(Some(f), Some(cfg));
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn no_file_returns_failed() {
        let out = run(None, None);
        assert!(matches!(out.result, OutputResultType::Failed));
    }

    #[test]
    fn on_load_and_unload_do_not_panic() {
        let plugin = UploadFileValidator;
        plugin.on_load();
        plugin.on_unload();
    }
}
```

> 删除原 `combine_all_three_dims`（多文件混合场景，单文件模型下无对应语义）与原 `all_rejected_returns_failed`（被 `pass_type_rejects_non_matching` 等覆盖）。

- [ ] **Step 7: 改 `pre_upload.rs` 模块声明**

Modify `file_uploader_plugins/src/pre_upload.rs`：

```rust
pub mod upload_file_validator;
```

- [ ] **Step 8: 迁移资源目录并改 meta.json**

Run:
```bash
git mv file_uploader_plugins/resources/pre/upload_file_filter file_uploader_plugins/resources/pre/upload_file_validator
```

Modify `file_uploader_plugins/resources/pre/upload_file_validator/meta.json`：

```json
{
  "name": "upload_file_validator",
  "title": "上传文件校验器",
  "description": "按文件类型(MIME)、文件名、大小三维校验单个上传文件（不通过则中断 pipeline）",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "PreUpload"
}
```

`config.json` 内容不动（仅目录名变了）。

- [ ] **Step 9: 验证 plugins crate 编译与测试**

Run: `cargo build -p file_uploader_plugins`
Expected: 编译通过。

Run: `cargo test -p file_uploader_plugins`
Expected: `default_input_handler::tests::*` 与 `upload_file_validator::tests::*` 全部 PASS（`download_network_real` 默认 `#[ignore]` 不计）。

> 注意：此时 `file_uploader_core`（main.rs 引用 `UploadFileFilter`、`file_list`）与 `uploader_example_plugin`（`file_list`）仍编译失败，属预期，Task 3 修复。

- [ ] **Step 10: Commit**

```bash
git add file_uploader_plugins/
git commit -m "refactor(plugins): 适配单文件 ctx + filter→validator 重命名

- default_input_handler: 去循环，run 返回 Option<Arc<UploadFileData>>
- upload_file_filter → upload_file_validator: 批量过滤→单文件校验
  (struct/name/资源目录/meta.json 全量重命名)
- 删除多文件批量断言用例，新增单文件 accept/reject 用例"
```

---

## Task 3: core（registry + main）+ example_plugin 适配

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs`
- Modify: `file_uploader_core/src/main.rs`
- Modify: `uploader_example_plugin/src/lib.rs`

- [ ] **Step 1: 改 `registry.rs` 的 `output_to_input`**

Modify `file_uploader_core/src/pipeline/registry.rs:162-184`：

```rust
    fn output_to_input(
        output: &UploadOutputCtx,
        source_ctx: &UploadInputCtx,
    ) -> UploadInputCtx {
        let mut extra_info = source_ctx.extra_info.clone().unwrap_or_default();
        if let Some(ref output_extra) = output.extra_info {
            for (k, v) in output_extra {
                extra_info.insert(k.clone(), v.clone());
            }
        }

        UploadInputCtx {
            file: output.file.clone(),
            config_info: Arc::new(None),
            extra_info: if extra_info.is_empty() {
                None
            } else {
                Some(extra_info)
            },
            related_process_info: source_ctx.related_process_info.clone(),
            work_dir: source_ctx.work_dir.clone(),
        }
    }
```

- [ ] **Step 2: 改 `registry.rs` 的 `execute_pipeline`（plugin_input + fail_ctx + 末尾兜底）**

Modify `file_uploader_core/src/pipeline/registry.rs:222-313`。

每插件 `plugin_input`（`:223-229`）：

```rust
                let plugin_input = UploadInputCtx {
                    file: current_ctx.file.clone(),
                    config_info: Arc::new(plugin.registry_config.clone()),
                    extra_info: current_ctx.extra_info.clone(),
                    related_process_info: current_ctx.related_process_info.clone(),
                    work_dir: current_ctx.work_dir.clone(),
                };
```

错误兜底 `fail_ctx`（`:246-251`）：

```rust
                        let fail_ctx = UploadOutputCtx {
                            result: file_uploader_sdk::models::enums::OutputResultType::Failed,
                            message: e.to_string(),
                            file: None,
                            extra_info: None,
                        };
```

末尾兜底返回（`:305-313`）——`current_ctx.file` 已是 `Option<Arc<...>>`，直接移动：

```rust
        match last_output {
            Some(output) => output,
            None => UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: String::new(),
                file: current_ctx.file,
                extra_info: current_ctx.extra_info,
            },
        }
```

- [ ] **Step 3: 改 `registry.rs` 测试中的 MockPlugin 返回值与构造点**

`registry.rs` 测试模块（`:317-1031`）有多处 `file_list` 引用，全部机械替换。

(a) `MockPlugin::execute`（`:336-343`）：
```rust
        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: "mock execute".to_string(),
                file: None,
                extra_info: None,
            }
        }
```

(b) `MockPluginWithLoadCounter::execute`（`:602-609`）：`file_list: None` → `file: None`。

(c) `FailPlugin::execute`（`:728-735`）：`file_list: None` → `file: None`。

(d) `ConfigReadPlugin::execute`（`:786-794`）：`file_list: None` → `file: None`。

(e) `CaptureWorkDir::execute`（`:983-991`）——原 `file_list: Some(ctx.file_list.clone())` 改为：
```rust
            fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
                *self.seen.lock().unwrap() = ctx.work_dir.clone();
                UploadOutputCtx {
                    result: file_uploader_sdk::models::enums::OutputResultType::Success,
                    message: "ok".into(),
                    file: ctx.file.clone(),
                    extra_info: None,
                }
            }
```

(f) 所有测试函数中的 `UploadInputCtx { file_list: vec![], ... }` 构造点（`:800-806` / `:820-826` / `:881-887` / `:957-963` / `:1022-1028`）：
```rust
        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            related_process_info: None,
            work_dir: None,
        };
```
（`:1022-1028` 的 `work_dir` 保持 `Some("/data/wd-flow".to_string())`，仅 `file_list: vec![]` → `file: None`。）

- [ ] **Step 4: 改 `main.rs`（字段 + filter 引用改名）**

Modify `file_uploader_core/src/main.rs`。

`:7` import 改名：
```rust
use file_uploader_plugins::pre_upload::upload_file_validator::UploadFileValidator;
```

`:20-26` 构造点：
```rust
    let ctx = UploadInputCtx {
        file: None,
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
        work_dir: None,
    };
```

`:44-52` filter 插件加载（目录名 + struct + 变量名 + 日志文案）：
```rust
    let validator_dir = target_dir.join("resources/pre/upload_file_validator");
    let Ok(validator_plugin) = UploadPluginInfo::new_in_process(
        validator_dir.to_str().unwrap(),
        Box::new(UploadFileValidator),
    ) else {
        error!("Validator plugin load error");
        return;
    };
    info!("Validator plugin loaded: {}", validator_plugin.id);
```

`:54-67` 执行段（变量名 `filter_dir`/`filter_plugin` → `validator_dir`/`validator_plugin`）：
```rust
    let in_process_dir = validator_dir.clone();
    let plugin = validator_plugin;
    info!("Plugin loaded: {}", plugin.id);
    let result = match plugin.slot.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            error!("Plugin execute error: {:?}", e);
            return;
        }
    };
    info!(
        "Plugin execute result: {:?}",
        serde_json::to_string(&result).unwrap_or("plugin error".to_string())
    );
```

- [ ] **Step 5: 改 `example_plugin/src/lib.rs`**

Modify `uploader_example_plugin/src/lib.rs:16-21`：

```rust
        UploadOutputCtxS {
            result: OutputResultType::Success,
            message: "成功".to_string().into(),
            file: stabby::option::Option::None(),
            extra_info: stabby::option::Option::None(),
        }
```

- [ ] **Step 6: 验证全 workspace 编译与测试**

Run: `cargo build`
Expected: 全 workspace（含 `uploader_example_plugin` cdylib）编译通过。

Run: `cargo test`
Expected: 全部测试 PASS（`download_network_real` 默认 ignore 不计）。

Run: `cargo run -p file_uploader_core`
Expected: 运行无 panic，日志输出 validator plugin 与 dylib plugin 的加载与执行结果。

- [ ] **Step 7: Commit**

```bash
git add file_uploader_core/src/pipeline/registry.rs file_uploader_core/src/main.rs uploader_example_plugin/src/lib.rs
git commit -m "refactor(core,example): 适配单文件 ctx

- registry: output_to_input/plugin_input/fail_ctx/末尾兜底同步 file 字段
- registry tests: MockPlugin 返回值与构造点适配
- main: file_list→file + UploadFileFilter→UploadFileValidator 引用改名
- example_plugin: UploadOutputCtxS.file 字段"
```

---

## Task 4: 文档同步（README + AGENTS.md + SKILL.md）

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `.agents/skills/designing-in-process-plugins/SKILL.md`

- [ ] **Step 1: 改 `README.md` 进程内插件示例**

Modify `README.md:135-140`（`file_list: None` → `file: None`）：

```rust
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        // 处理逻辑...
        UploadOutputCtx {
            result: OutputResultType::Success,
            message: "done".to_string(),
            file: None,
            extra_info: None,
        }
    }
```

- [ ] **Step 2: 改 `README.md` dylib 插件示例**

Modify `README.md:198-203`（`file_list` → `file`）：

```rust
        UploadOutputCtxS {
            result: OutputResultType::Success,
            message: "成功".to_string().into(),
            file: stabby::option::Option::None(),
            extra_info: stabby::option::Option::None(),
        }
```

> 若 `README.md` 其他位置（如数据流说明）提及 `file_list`，一并改为 `file`。

- [ ] **Step 3: 改 `AGENTS.md` 数据流与 pipeline 描述**

Modify `AGENTS.md`。将「核心数据流」段与「Pipeline 执行」段中提及 `file_list` 的描述改为 `file`（单文件语义）。具体：
- 「插件输出通过 `output_to_input` 转换为下一插件输入（`extra_info` 累积传递）」段：若含 `file_list` 字样 → `file`。

> 用 `rg "file_list" AGENTS.md` 确认无遗漏；若文档表述不直接引用字段名（仅描述"文件数据传递"），可保留自然语言不动。

- [ ] **Step 4: 改 `SKILL.md`（designing-in-process-plugins）**

Modify `.agents/skills/designing-in-process-plugins/SKILL.md`。

`:74` 注释（`ctx.file_list` → `ctx.file`）：
```rust
        // 处理 ctx.file（Option<Arc<UploadFileData>>），返回 UploadOutputCtx
```

`:99` 关联函数引用（`success_files` → `success_file`）：
```
- 正常：`UploadOutputCtx::success_file(msg, file)`（`success(msg)` 无文件产出）
- 过滤/校验类插件，校验不通过 → `UploadOutputCtx::failed(msg)`（会中断 pipeline）
```

`:104`「空列表用 `Failed`」表述改为「校验不通过用 `Failed`」。

`:124` 测试构造描述（`含 config_info 与 file_list` → `含 config_info 与 file`）：
```
单元测试置于插件文件内 `#[cfg(test)] mod tests`。构造 `UploadInputCtx`（含 `config_info` 与 `file`），断言 `execute` 输出。覆盖：正常路径、边界（`file` 为 None）、`config_info` 为 None。参考 `upload_file_validator.rs` 的测试组织。
```

`:149` 范例引用（`file_type_filter.rs` → `upload_file_validator.rs`，修正文档与实际文件名长期不一致，并更新描述为单文件校验语义）：
```
`file_uploader_plugins/src/pre_upload/upload_file_validator.rs` —— 按 `pass_type`/`reject_type`/`pass_name`/`max_size` 四维校验单个文件，不通过返回 `Failed`，含完整单元测试。该插件使用 `config_util::get_list` 与 `UploadOutputCtx::failed/success_file`，可作为新工具用法的范例。
```

`:174` 快速检查清单（`success_files` → `success_file`）：
```
- [ ] 构造输出走 `UploadOutputCtx` 关联函数（success/failed/interrupt/success_file）
```

- [ ] **Step 5: 全局复查无残留 `file_list` / `success_files`**

Run: `rg "file_list|success_files" --type rust --type markdown`
Expected: 无业务代码/文档残留（仅可能在 `docs/superpowers/specs|plans` 历史文档中出现，属历史记录，不改）。

若有业务代码残留，回到对应 Task 修复。

- [ ] **Step 6: Commit**

```bash
git add README.md AGENTS.md .agents/skills/designing-in-process-plugins/SKILL.md
git commit -m "docs: 同步单文件 ctx 重构

- README/AGENTS/SKILL: file_list→file, success_files→success_file
- SKILL: filter→validator 描述与范例文件名修正"
```

---

## 验证（最终）

- [ ] `cargo build`（全 workspace，含 cdylib）通过
- [ ] `cargo test` 全绿
- [ ] `cargo run -p file_uploader_core` 无 panic
- [ ] `rg "file_list|success_files"` 业务代码与文档无残留
