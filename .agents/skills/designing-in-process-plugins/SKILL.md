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
        // 处理 ctx.file_list，返回 UploadOutputCtx
        todo!()
    }

    fn on_load(&self) { tracing::info!("size_limiter: loading"); }
    fn on_unload(&self) { tracing::info!("size_limiter: unloading"); }
}
```

> **易遗漏**：`phase()` 是必须方法，漏了会编译报 `missing phase in implementation`。`on_load`/`on_unload` 虽有默认空实现，但建议显式实现（至少 log），便于观测插件生命周期。

### 3. 配置读取约定

运行期配置经 `registry_config → ctx.config_info`（裸 `serde_json::Value`）注入。结构约定为 `{ "<param_key>": <default_value 同型值>, ... }`，与 `config.json` params 的 key 对齐。`config_info` 为 `None`（未注册配置）→ 用默认/空，插件须容忍。

### 4. 输出约定

- 正常：`file_list = Some(处理后)`，`result = Success`，`message` 记统计
- 过滤/校验类插件，若处理后列表为空 → `result = Failed`（会中断 pipeline）
- `extra_info: Option<HashMap<String,String>>` 可累积传递给下游插件

### 5. 模块注册

`file_uploader_plugins/src/<phase_dir>.rs` 加 `pub mod <name>;`（如 `pub mod size_limiter;`）。`<phase_dir>` ∈ `pre_upload` / `upload` / `post_upload`。

### 6. 测试（TDD）

单元测试置于插件文件内 `#[cfg(test)] mod tests`。构造 `UploadInputCtx`（含 `config_info` 与 `file_list`），断言 `execute` 输出。覆盖：正常路径、边界（空列表）、`config_info` 为 None。参考 `file_type_filter.rs` 的测试组织。

### 7. 注册到 pipeline

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

`file_uploader_plugins/src/pre_upload/file_type_filter.rs` —— 按 `pass_type`/`reject_type` 用 glob 通配符过滤，处理后为空返回 `Failed`，含完整单元测试。

## 常见错误

| 症状 | 原因 / 修法 |
|---|---|
| `not all trait items implemented, missing phase` | impl 缺 `phase()`，补 `fn phase(&self) -> UploadPhase` |
| `Failed to open meta file ...` | resource_dir 路径错；或 build.rs 未把 resources 复制到 target；确认路径指向 `target/resources/<phase>/<name>` |
| 运行时 `config_info` 为 None | 注册时未传 `registry_config`（第 4 参数） |
| 新增/改了资源不生效 | 改的是源 `resources/`，运行读的是 `target/resources/` 副本；`cargo:rerun-if-changed=resources` 已覆盖整树，重新 build 即可 |
| id 含意外的 phase 形态 | id 用 `format!("{:?}", phase)` → `PreUpload`（非小写） |

## 快速检查清单

- [ ] `meta.json` 含 name/title/version/description/author/phase
- [ ] `config.json`：access 三权限点 + params（每项含 `form`）
- [ ] `impl UploadPlugin`：name/phase/execute + on_load/on_unload（log）
- [ ] `src/<phase_dir>.rs` 注册 `pub mod <name>;`
- [ ] 单元测试覆盖（含 config_info 为 None）
- [ ] `new_in_process` 路径指向 `target/resources/<phase>/<name>`
- [ ] 注册时 `registry_config` 传入实际运行值
