---
name: designing-in-process-plugins
description: Use when creating, implementing, or refactoring an in-process plugin (UploadPlugin) in file-uploader-x. Triggers include "not all trait items implemented, missing phase" compile error, plugin load failure ("Failed to open meta file"), config not taking effect, resources not copied to target, or adding a new pre/upload/post stage plugin.
---

# 设计并实现进程内插件

## Overview

进程内插件实现 `UploadPlugin` trait（`file_uploader_sdk::models::interface`），在 `UploadPhase` 某阶段执行。一个插件 = **资源目录**（`meta.json` + `config.json`）+ **Rust 实现** + **模块注册** + **build.rs 复制**（已统一）+ **pipeline 注册**。

## When to Use

- 新增一个进程内插件（任意阶段）
- 为已有插件加配置项 / 权限
- 排查插件加载失败、配置未生效、资源未复制等问题

## 必备文件清单（缺一不可）

| 文件 | 作用 |
|---|---|
| `file_uploader_plugins/resources/<phase>/<name>/meta.json` | 元数据（name/title/version/description/author/phase） |
| `.../config.json` | 表单驱动配置 schema（access + params） |
| `file_uploader_plugins/src/<phase_dir>/<name>.rs` | `impl UploadPlugin` |
| `file_uploader_plugins/src/<phase_dir>.rs` | `pub mod <name>;` |
| `file_uploader_plugins/build.rs` | 递归复制 `resources/`（已存在，新增插件自动覆盖） |

`<phase>` 段：`pre`→PreUpload、`upload`→Upload、`post`→PostUpload。

## 实现步骤

### 1. 资源目录

`meta.json`：
```json
{
  "name": "size_limiter", "title": "体积限制", "description": "限制单文件体积",
  "version": "0.0.1", "author": "you", "phase": "PreUpload"
}
```

`config.json`（表单驱动 schema）：
```json
{
  "access": { "fs_read": false, "fs_write": false, "network": false },
  "params": [
    {
      "key": "max_size", "title": "最大体积", "description": "字节",
      "config_type": "Custom", "default_value": 0,
      "form": { "type": "text" }
    }
  ]
}
```
- `form`：`{ "type": "text", "secret": bool }` 或 `{ "type": "select", "options": [{label,value}], "multiple": bool, "allow_custom": bool }`（字段均可省略，走 serde default）
- `access`：`fs_read`/`fs_write`/`network`，每项 `true`/`false` 或 `["路径/host"]` 白名单；默认 Deny；**纯透传不做执行逻辑**

### 2. Rust 实现

必须方法：`name` / `phase` / `execute`；生命周期钩子：`on_load` / `on_unload`（trait 有默认空实现，建议显式 log 标记加载/卸载）。

```rust
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;

pub struct SizeLimiter;

impl UploadPlugin for SizeLimiter {
    fn name(&self) -> &'static str { "size_limiter" }
    fn phase(&self) -> UploadPhase { UploadPhase::PreUpload }
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        // 从 ctx.config_info（Arc<Option<Value>>）读运行期配置
        // 处理 ctx.file（Option<Arc<UploadFileData>>），返回 UploadOutputCtx
        todo!()
    }

    fn on_load(&self) { tracing::info!("size_limiter: loading"); }
    fn on_unload(&self) { tracing::info!("size_limiter: unloading"); }
}
```

> **易遗漏**：`phase()` 是必须方法，漏了会编译报 `missing phase in implementation`。`on_load`/`on_unload` 虽有默认空实现，但建议显式实现（至少 log），便于观测插件生命周期。

### 3. 配置读取约定

运行期配置经 `registry_config → ctx.config_info`（裸 `serde_json::Value`）注入。结构约定为 `{ "<param_key>": <default_value 同型值>, ... }`，与 `config.json` params 的 key 对齐。`config_info` 为 `None`（未注册配置）→ 用默认/空，插件须容忍。

**优先使用 SDK helper**（`file_uploader_sdk::utils::config_util`），不要手写 `Value` 解析：

- `config_util::get_str(&ctx.config_info, "key") -> Option<String>`
- `config_util::get_bool(&ctx.config_info, "key") -> Option<bool>`
- `config_util::get_list(&ctx.config_info, "key") -> Vec<String>`（数组→字符串列表，缺失/非数组→空）

### 4. 输出约定

**优先使用 `UploadOutputCtx` 关联函数**构造输出，避免手写字面量：

- 正常：`UploadOutputCtx::success_file(msg, file)`（`success(msg)` 无文件产出）
- 过滤/校验类插件，校验不通过 → `UploadOutputCtx::failed(msg)`（会中断 pipeline）
- 中断：`UploadOutputCtx::interrupt(msg)`
- `extra_info` 需传递时仍可先构造再赋值字段

行为约定：正常 `result = Success`、`message` 记统计；校验不通过用 `Failed`；`extra_info: Option<HashMap<String,String>>` 可累积传递给下游插件。

### 5. 活动目录与文件 IO

`ctx.work_dir: Option<String>` 是本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传，`None`=未设置）。插件需读写中间文件时**必须走 `file_uploader_sdk::utils::fs_util`**，**禁止直接用 `std::fs`**：

- 写文件：`fs_util::write(ctx.work_dir.as_deref().unwrap_or(""), ext, reader) -> Result<(文件名, 路径)>` —— **强制使用生成的唯一文件名**（`{ts}_{hash}.{ext}`），不支持调用方指定文件名；自定义命名用 `fs_util::write_with_gen(.., gen_fn: fn(&str)->String)`。
- 读文件：`fs_util::open_read(work_dir, filename) -> Result<BufReader<File>>`（流式）；便捷：`read_to_end` / `read_to_string`。
- 其它：`fs_util::create_work_dir(base, id)`（宿主侧创建活动目录）、`fs_util::resolve`（沙箱路径校验）、`fs_util::exists` / `fs_util::create_dir`。

错误：`work_dir` 为空/`None` 调 IO → `UploadError::WorkDirNotSet`；`../` 或绝对路径越界 → `WorkDirPathEscape`。

**强制力边界**：dylib 插件直接调 libc / `std::fs` 无法被拦截，沙箱仅覆盖「走 `fs_util`」的路径；`BufReader<File>` 为 std 类型，不跨 dylib ABI 边界传递。未来会结合 `config.json` 的 `access`（fs_read/fs_write）与 work_dir 做权限收敛（当前预留未实现）。

### 6. 模块注册

`file_uploader_plugins/src/<phase_dir>.rs` 加 `pub mod <name>;`（如 `pub mod size_limiter;`）。`<phase_dir>` ∈ `pre_upload` / `upload` / `post_upload`。

### 7. 测试（TDD）

单元测试置于插件文件内 `#[cfg(test)] mod tests`。构造 `UploadInputCtx`（含 `config_info` 与 `file`），断言 `execute` 输出。覆盖：正常路径、边界（`file` 为 None）、`config_info` 为 None。参考 `upload_file_validator.rs` 的测试组织。

构造 `UploadInputCtx` 时须补 `work_dir` 字段（不涉及文件 IO 的插件测试用 `None`；涉及者用临时目录）。

### 8. 注册到 pipeline

```rust
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_core::pipeline::registry::{PluginRegistryInfo, PluginRegistryStatus, UploadPluginRegistryTable};

let info = UploadPluginInfo::new_in_process("./resources/pre/size_limiter", Box::new(SizeLimiter))?;
let reg = PluginRegistryInfo::new(
    std::sync::Arc::new(info),
    1,                                          // priority：值越小越先
    PluginRegistryStatus::Enable,
    Some(serde_json::json!({"max_size": 1048576})),  // registry_config：本次运行的实际配置值
);
let table = UploadPluginRegistryTable::new("pipeline_id".into(), vec![reg]);
let out = table.execute_pipeline(input_ctx, None);
```
- id 自动生成：`in_process_{phase:?}_{name}`（如 `in_process_PreUpload_size_limiter`）
- `registry_config` 是运行期实际值（覆盖 config.json 的 `default_value`）；不传则 `config_info` 为 None

## 范例

`file_uploader_plugins/src/pre_upload/upload_file_validator.rs` —— 按 `pass_type`/`reject_type`/`pass_name`/`max_size` 四维校验单个文件，不通过返回 `Failed`，含完整单元测试。该插件使用 `config_util::get_list` 与 `UploadOutputCtx::failed/success_file`，可作为新工具用法的范例。

## 常见错误

| 症状 | 原因 / 修法 |
|---|---|
| `not all trait items implemented, missing phase` | impl 缺 `phase()`，补 `fn phase(&self) -> UploadPhase` |
| `Failed to open meta file ...` | resource_dir 路径错；或 build.rs 未把 resources 复制到 target；确认路径指向 `target/resources/<phase>/<name>` |
| 运行时 `config_info` 为 None | 注册时未传 `registry_config`（第 4 参数） |
| 新增/改了资源不生效 | 改的是源 `resources/`，运行读的是 `target/resources/` 副本；`cargo:rerun-if-changed=resources` 已覆盖整树，重新 build 即可 |
| id 含意外的 phase 形态 | id 用 `format!("{:?}", phase)` → `PreUpload`（非小写） |
| `WorkDirNotSet` | `ctx.work_dir` 为 `None`（或空串）时调用 `fs_util` IO；先确认宿主已透传 work_dir |
| `WorkDirPathEscape` | 传入 `../` 或绝对路径越出 work_dir；仅用 `fs_util` 生成的唯一名或在 work_dir 内的相对路径 |
| dylib 加载/运行 ABI 不兼容 | stabby 结构体增字段为破坏性 ABI 变更，宿主与 dylib 须同版本重编（重新 `cargo build -p uploader_example_plugin`） |

## 快速检查清单

- [ ] `meta.json` 含 name/title/version/description/author/phase
- [ ] `config.json`：access 三权限点 + params（每项含 `form`）
- [ ] `impl UploadPlugin`：name/phase/execute + on_load/on_unload（log）
- [ ] `src/<phase_dir>.rs` 注册 `pub mod <name>;`
- [ ] 单元测试覆盖（含 config_info 为 None）
- [ ] `new_in_process` 路径指向 `target/resources/<phase>/<name>`
- [ ] 注册时 `registry_config` 传入实际运行值
- [ ] 读配置走 `config_util`（不手写 `Value` 解析）
- [ ] 构造输出走 `UploadOutputCtx` 关联函数（success/failed/interrupt/success_file）
- [ ] 读写中间文件走 `fs_util`（不直接用 `std::fs`）
- [ ] 测试构造 `UploadInputCtx` 时补 `work_dir` 字段
