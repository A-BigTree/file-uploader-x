---
name: designing-in-process-plugins
description: Use when creating, implementing, or refactoring an in-process plugin (UploadPlugin) in file-uploader-x. Triggers include "not all trait items implemented, missing phase" compile error, plugin load failure ("Failed to open meta file"), config not taking effect, resources not copied to target, config validation errors (GroupMissing / Required / TypeMismatch), or adding a new input/pre/upload/post stage plugin.
---

# 设计并实现进程内插件

## Overview

进程内插件实现 `UploadPlugin` trait（`file_uploader_sdk::models::interface`），在 `UploadPhase` 某阶段执行。

一个插件 = **资源目录**（`meta.json` + `config.json` + `README.md`）+ **Rust 实现** + **模块注册** + **build.rs 复制**（已统一）+ **pipeline 注册**。

## When to Use

- 新增一个进程内插件（任意阶段）
- 为已有插件加配置项 / 权限 / 分组
- 排查插件加载失败、配置未生效、配置校验报错、资源未复制等问题

## 必备文件清单

| 文件 | 必需 | 作用 |
|---|---|---|
| `file_uploader_plugins/resources/<phase>/<name>/meta.json` | 是 | 元数据（name/title/version/description/author/phase） |
| `.../config.json` | 否 | 配置 schema（access + common + groups）；缺失→空容器 |
| `.../README.md` | 否 | **面向配置者的使用说明书**（只记录路径，不加载内容） |
| `file_uploader_plugins/src/<phase_dir>/<name>.rs` | 是 | `impl UploadPlugin` |
| `file_uploader_plugins/src/<phase_dir>.rs` | 是 | `pub mod <name>;` |
| `file_uploader_plugins/build.rs` | 已存在 | 递归复制 `resources/` 整树（新增插件自动覆盖） |

`<phase>` 段：`input`→Input、`pre`→PreUpload、`upload`→Upload、`post`→PostUpload。

## 实现步骤

### 1. 资源目录

`meta.json`：

```json
{
  "name": "size_limiter", "title": "体积限制", "description": "限制单文件体积",
  "version": "0.0.1", "author": "you", "phase": "PreUpload"
}
```

`config.json` —— **三段结构**：`access`（权限）+ `common`（公共参数）+ `groups`（互斥分组）。

**单形态插件**（绝大多数）：只写 `common`，`groups` 留空数组。

```json
{
  "access": { "fs_read": false, "fs_write": false, "network": false },
  "common": [
    {
      "key": "max_size", "title": "最大体积", "description": "支持 10mb 这类写法",
      "config_type": "Custom", "default_value": "0", "required": false,
      "form": { "type": "text", "max_len": 16, "pattern": "^\\s*\\d+\\s*(?i:b|kb|mb|gb|tb)?\\s*$" }
    },
    {
      "key": "strict_mode", "title": "严格模式", "description": "识别不出类型就拒",
      "config_type": "Default", "default_value": false, "required": false,
      "form": { "type": "switch" }
    }
  ],
  "groups": []
}
```

**多形态插件**（有互斥工作模式，如 oss / s3 / local）：`common` 放共用参数，`groups` 每项一种模式。

```json
{
  "access": { "fs_write": true, "network": true },
  "common": [
    {
      "key": "retry_times", "title": "重试次数", "config_type": "Default",
      "default_value": 3, "form": { "type": "number", "min": 0, "max": 10, "integer": true }
    }
  ],
  "groups": [
    {
      "group": "oss", "title": "阿里云 OSS", "description": "传到对象存储",
      "params": [
        {
          "key": "endpoint", "title": "Endpoint", "config_type": "Default",
          "default_value": "", "required": true,
          "form": { "type": "text", "min_len": 8, "max_len": 256, "pattern": "^https?://.+" }
        },
        {
          "key": "access_secret", "title": "AccessKey Secret", "config_type": "Default",
          "default_value": "", "required": true,
          "form": { "type": "text", "secret": true, "max_len": 256 }
        }
      ]
    },
    {
      "group": "local", "title": "本地存储",
      "params": [
        {
          "key": "base_dir", "title": "存储根目录", "config_type": "Default",
          "default_value": "/tmp/uploads", "required": true,
          "form": { "type": "text", "pattern": "^/.*" }
        },
        {
          "key": "naming", "title": "命名策略", "config_type": "Default",
          "default_value": "uuid", "required": true,
          "form": {
            "type": "select", "multiple": false, "allow_custom": false,
            "options": [
              { "label": "原始文件名", "value": "origin" },
              { "label": "UUID", "value": "uuid" }
            ]
          }
        }
      ]
    }
  ]
}
```

**分组语义**：`group` 是**互斥的模式类型**，运行态**只激活一个**。激活 `oss` 时 `local` 的参数完全不参与校验。
`groups` 非空 → 运行态配置**必须**带 `group`；`groups` 为空 → 运行态配置**不应**出现 `group`。

**`access`**：`fs_read` / `fs_write` / `network`，每项 `true` / `false` 或 `["路径 或 host"]` 白名单；默认 Deny；**纯透传不做执行逻辑**。

### 2. 四种表单控件与声明式约束

`form` 用 `type` 区分控件，控件级约束写在同一个对象里：

| `type` | 期望值类型 | 可用约束 |
|---|---|---|
| `text` | string | `secret`（密码框，不回显）/ `min_len` / `max_len` / `pattern`（正则）/ `placeholder` |
| `switch` | bool | 无 |
| `select` | 标量（单选）或数组（多选） | `options: [{label,value}]` / `multiple` / `allow_custom` / `min_items` / `max_items` |
| `number` | number | `min` / `max` / `step` / `integer` |

`PluginConfigItem` 顶层还有通用约束 `required`（缺失 / `null` / `""` / `[]` 均视为未填）。
所有约束字段均可省略，走 serde default。

**选型要点**

- 布尔值用 `switch`，**不要**用 `text` 存 `"true"` 字符串
- 数字用 `number`，**不要**用 `text`（`10mb` 这类带单位的值例外，用 `text` + `pattern`）
- 密钥、token 一律 `text` + `"secret": true`
- 固定候选项用 `select` + `allow_custom: false`（框架会校验值必须在 `options` 内）
- 开放式多选（如 MIME 列表）用 `select` + `multiple: true` + `allow_custom: true`

### 3. Rust 实现

必须方法：`name` / `phase` / `execute`。
可选（均有默认实现，建议按需覆盖）：`on_load` / `on_unload` / `validate_params`。

```rust
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;
use file_uploader_sdk::models::interface::UploadPlugin;
use file_uploader_sdk::utils::config_util;

pub struct SizeLimiter;

impl UploadPlugin for SizeLimiter {
    fn name(&self) -> &'static str { "size_limiter" }
    fn phase(&self) -> UploadPhase { UploadPhase::PreUpload }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        let max_size = config_util::get_size(&ctx.config_info, "max_size");
        let strict = config_util::get_bool(&ctx.config_info, "strict_mode").unwrap_or(false);
        let Some(f) = ctx.file.as_ref() else {
            return UploadOutputCtx::failed("size_limiter: no file");
        };
        // ... 判定 ...
        UploadOutputCtx::success_file("size_limiter: accepted", f.clone())
    }

    /// 业务级入参校验（框架声明式约束通过后才会调用）
    fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
        if let Some(raw) = config_util::get_str(&ctx.config_info, "max_size") {
            let t = raw.trim();
            if !t.is_empty() && config_util::parse_size(t).is_none() {
                return Err(format!("max_size 无法解析: '{raw}'"));
            }
        }
        Ok(())
    }

    fn on_load(&self) { tracing::info!("size_limiter: loading"); }
    fn on_unload(&self) { tracing::info!("size_limiter: unloading"); }
}
```

> **易遗漏**：`phase()` 是必须方法，漏了会编译报 `missing phase in implementation`。

### 4. 两层参数校验

**框架声明式校验先跑，通过后才调插件 `validate_params`。**

**声明式校验（框架自动做，写在 `config.json` 里）**
覆盖：`required` 判空、值类型与控件匹配、文本长度与正则、数值范围与步进、`select` 候选项合法性与数量、
`group` 存在性与合法性。错误全部累积不短路。

**插件 `validate_params`（写代码，只做声明式表达不了的事）**

适合放这里的：

- 跨字段一致性（如 A 开启时 B 必填）
- 需要调用库才能判定的格式（如 glob 模式合法性、`parse_size` 可解析性）
- 业务白名单（如 `group` 只支持已实现的几种）

**不要**在这里重复做 required / 长度 / 正则 / 范围校验 —— 那些写进 `config.json` 即可。

**校验时机**（宿主侧，插件作者了解即可）

| 时机 | 做什么 |
|---|---|
| `UploadPluginRegistryTable::new` | 只跑声明式校验，**不加载插件**；错误缓存到 `declarative_errors()` 并 warn |
| `try_new` | 声明式校验失败即返回 `Err(Vec<(plugin_id, ValidationError)>)` |
| `preload_all` | 加载插件后**追加**插件级 `validate_params` |
| `validate_all()` | 声明式 + 插件级全量校验（会加载全部插件） |
| `validate_plugin_config(&config, &values)` | 对外纯函数 API，供宿主/前端保存配置前预校验 |

### 5. 配置读取约定

运行期配置经 `registry_config → ctx.config_info` 注入，形状为**扁平一层 JSON**，
保留字段 `group` 标识激活的分组，其余为参数 KV：

```json
{ "group": "oss", "retry_times": 3, "endpoint": "https://...", "access_secret": "SK" }
```

`config_info` 为 `None`（未注册配置）→ 用默认/空，插件须容忍。
**`default_value` 不做合并**，仅供 UI 预填；插件必须自行 `unwrap_or` 兜底。

**优先使用 SDK helper**（`file_uploader_sdk::utils::config_util`），不要手写 `Value` 解析：

| helper | 返回 | 用途 |
|---|---|---|
| `get_group(&cfg)` | `Option<String>` | 读激活的分组标识 |
| `get_str(&cfg, k)` | `Option<String>` | `text` 控件 |
| `get_bool(&cfg, k)` | `Option<bool>` | `switch` 控件 |
| `get_i64(&cfg, k)` | `Option<i64>` | `number` + `integer: true` |
| `get_f64(&cfg, k)` | `Option<f64>` | `number` 浮点 |
| `get_list(&cfg, k)` | `Vec<String>` | `select` + `multiple`（非数组/缺失→空） |
| `get_size(&cfg, k)` | `Option<u64>` | 带单位的体积（字符串走 `parse_size`，数字走 `as_u64`） |
| `parse_size(s)` | `Option<u64>` | 单独解析 `10mb` 这类字符串 |

多形态插件按分组分派：

```rust
match config_util::get_group(&ctx.config_info).as_deref() {
    Some("oss")   => self.upload_oss(ctx),
    Some("local") => self.save_local(ctx),
    other => UploadOutputCtx::failed(format!("不支持的分组: {other:?}")),
}
```

### 6. 输出约定

**优先使用 `UploadOutputCtx` 关联函数**构造输出，避免手写字面量：

- 正常：`UploadOutputCtx::success_file(msg, file)`（`success(msg)` 无文件产出）
- 校验不通过 → `UploadOutputCtx::failed(msg)`（**会中断 pipeline**）
- 中断：`UploadOutputCtx::interrupt(msg)`
- `extra_info` 需传递时仍可先构造再赋值字段

失败时的 `message` **建议打印实际生效的配置**，便于使用方对照排错。

> **安全提醒**：不要把整个 ctx 序列化后打日志 —— 会泄漏 `secret: true` 字段的值。
> 只打印必要的非敏感字段。

### 7. 活动目录与文件 IO

`ctx.work_dir: Option<String>` 是本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传，`None`=未设置）。
插件需读写中间文件时**必须走 `file_uploader_sdk::utils::fs_util`**，**禁止直接用 `std::fs`**：

- 写文件：`fs_util::write(work_dir, ext, reader) -> Result<(文件名, 路径)>` —— **强制使用生成的唯一文件名**（`{ts}_{hash}.{ext}`）；自定义命名用 `fs_util::write_with_gen(.., gen_fn)`
- 读文件：`fs_util::open_read(work_dir, filename) -> Result<BufReader<File>>`（流式）；便捷：`read_to_end` / `read_to_string`
- 其它：`fs_util::create_work_dir(base, id)`、`fs_util::resolve`（沙箱路径校验）、`fs_util::exists` / `create_dir` / `import_file` / `file_size` / `read_external_head`

错误：`work_dir` 为空/`None` 调 IO → `UploadError::WorkDirNotSet`；`../` 或绝对路径越界 → `WorkDirPathEscape`。

**强制力边界**：dylib 插件直接调 libc / `std::fs` 无法被拦截，沙箱仅覆盖「走 `fs_util`」的路径。
未来会结合 `config.json` 的 `access` 与 work_dir 做权限收敛（当前预留未实现）。

### 8. 模块注册

`file_uploader_plugins/src/<phase_dir>.rs` 加 `pub mod <name>;`。
`<phase_dir>` ∈ `input` / `pre_upload` / `upload` / `post_upload`。

### 9. README.md（可选但强烈建议）

**定位：面向配置者的使用说明书，不是开发文档。**
读者是「要用这个插件的人」，关心「这插件能帮我做什么、参数怎么填、填完会发生什么」。

推荐章节：

```
# <插件中文名>
一句话说清它解决什么问题

## 能做什么      —— 用大白话列举能力，不谈实现
## 什么时候用它  —— 典型场景表：场景 → 怎么设
## 怎么配        —— 按「目的」分小节，每节讲清填什么 + 效果是什么
## 参数一览      —— 表格：参数 | 作用 | 默认 | 怎么填
## 常见问题      —— Q&A，覆盖真实会踩的坑与排查顺序
```

**写作要求**

- 用参数的**中文标题**（如「允许类型」），不要用代码里的 `key`
- 讲「效果」不讲「机制」：写「png 通过、pdf 被拒」，不写「命中 glob 后进入 keep 判定」
- 多形态插件要说清各模式**互斥**，以及切换模式后原参数不再生效
- **不要**写：输入输出契约、`ctx` 字段名、Rust 类型名、trait 方法、错误码表、变更记录、阶段枚举
- 权限、所属阶段已在 `meta.json` / `config.json` 声明，README 不必重复

### 10. 测试（TDD）

单元测试置于插件文件内 `#[cfg(test)] mod tests`。构造 `UploadInputCtx`（含 `config_info` / `file` / `work_dir`），
断言 `execute` 与 `validate_params` 输出。

必覆盖：

- 正常路径
- 边界：`file` 为 `None`
- `config_info` 为 `None`（未注册配置）
- 每个新增参数的开 / 关或边界值
- `validate_params` 的通过态与每种拒绝态
- 多形态插件：每个分组各一条，外加未知分组

参考 `upload_file_validator.rs` 的测试组织。

### 11. 注册到 pipeline

```rust
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_core::pipeline::registry::{
    PluginRegistryInfo, PluginRegistryStatus, UploadPluginRegistryTable,
};

let info = UploadPluginInfo::new_in_process("./resources/pre/size_limiter", Box::new(SizeLimiter))?;
let reg = PluginRegistryInfo::new(
    std::sync::Arc::new(info),
    1,                                   // priority：值越小越先
    PluginRegistryStatus::Enable,
    Some(serde_json::json!({             // registry_config：扁平一层
        "max_size": "10mb",
        "strict_mode": true
    })),
);

// 严格构建：配置不合法直接失败
let table = UploadPluginRegistryTable::try_new("pipeline_id".into(), vec![reg])
    .map_err(|errs| format!("config invalid: {errs:?}"))?;

// 预加载 + 插件级校验
table.preload_all().map_err(|errs| format!("{errs:?}"))?;

let out = table.execute_pipeline(input_ctx, None);
```

- id 自动生成：`in_process_{phase:?}_{name}`（如 `in_process_PreUpload_size_limiter`）
- `registry_config` 是运行期实际值；不传则 `config_info` 为 `None`
- 宽松构建用 `new`（只 warn 不失败），之后可读 `table.declarative_errors()` 拿结构化明细

## 范例

- `file_uploader_plugins/src/pre_upload/upload_file_validator.rs` —— 单形态插件完整范例：
  五个参数（含 `switch`）、`validate_params` 业务校验、完整单元测试
- `file_uploader_plugins/src/input/default_input_handler.rs` —— `switch` + `number` 参数、`fs_util` 沙箱 IO
- `uploader_example_plugin/config.json` —— **多形态（common + groups）配置样板**，四种控件齐全
- 三份 `README.md`（两个内置插件 + example）—— 使用说明书写法范例

## 常见错误

| 症状 | 原因 / 修法 |
|---|---|
| `not all trait items implemented, missing phase` | impl 缺 `phase()`，补 `fn phase(&self) -> UploadPhase` |
| `Failed to open meta file ...` | resource_dir 路径错；或 build.rs 未把 resources 复制到 target；确认路径指向 `target/<profile>/resources/<phase>/<name>` |
| 运行时 `config_info` 为 `None` | 注册时未传 `registry_config`（第 4 参数） |
| 新增/改了资源不生效 | 改的是源 `resources/`，运行读的是 `target/.../resources/` 副本；重新 build 即可 |
| 校验报 `GroupMissing` | `config.json` 的 `groups` 非空，但 `registry_config` 没带 `group` |
| 校验报 `GroupNotAllowed` | `groups` 为空却传了 `group` 字段，去掉即可 |
| 校验报 `TypeMismatch: expected bool, found string` | `switch` 控件传了 `"true"` 字符串，改成 `true` |
| 校验报 `NotInOptions` | `select` 且 `allow_custom: false`，值不在 `options` 内；要么改值，要么开 `allow_custom` |
| 校验报 `Required` 但字段填了 | 填的是 `""` / `[]` / `null` —— 这三者都算未填 |
| 校验报 `InvalidPattern` | `config.json` 里的 `pattern` 正则本身写错了（注意 JSON 中反斜杠要写 `\\`） |
| 分组参数的校验没生效 | 该参数属于未激活的分组；只有 `common` + 激活分组的参数参与校验 |
| `WorkDirNotSet` | `ctx.work_dir` 为 `None`/空串时调用 `fs_util` IO；先确认宿主已透传 work_dir |
| `WorkDirPathEscape` | 传入 `../` 或绝对路径越出 work_dir；仅用 `fs_util` 生成的唯一名或 work_dir 内相对路径 |
| dylib 加载/运行 ABI 不兼容 | 给 `UploadDylibPlugin` 新增方法或改 stabby 结构体均为破坏性 ABI 变更；新方法须追加在 trait 末尾，并 `cargo build --workspace` 全量重编 |

## 快速检查清单

**资源**
- [ ] `meta.json` 含 name/title/version/description/author/phase
- [ ] `config.json`：`access` + `common` + `groups` 三段（单形态时 `groups: []`）
- [ ] 每个参数含 `key`/`title`/`config_type`/`default_value`/`required`/`form`
- [ ] 控件选型正确：布尔用 `switch`、数值用 `number`、密钥用 `text` + `secret`
- [ ] 能用声明式约束表达的（required/长度/正则/范围/候选项）都写进了 `config.json`
- [ ] `README.md` 为使用说明书（中文参数名、讲效果不讲机制、含常见问题）

**代码**
- [ ] `impl UploadPlugin`：name/phase/execute + on_load/on_unload（log）
- [ ] 只在 `validate_params` 里做声明式表达不了的校验（跨字段、库依赖、业务白名单）
- [ ] 读配置走 `config_util`（不手写 `Value` 解析）；多形态用 `get_group` 分派
- [ ] 参数缺失时有 `unwrap_or` 兜底（default_value 不会自动合并）
- [ ] 构造输出走 `UploadOutputCtx` 关联函数；失败 message 带上生效配置
- [ ] 日志不整体序列化 ctx（避免泄漏 `secret` 字段）
- [ ] 读写中间文件走 `fs_util`（不直接用 `std::fs`）
- [ ] `src/<phase_dir>.rs` 注册 `pub mod <name>;`

**测试与接入**
- [ ] 单测覆盖：正常路径、`file` 为 None、`config_info` 为 None、各参数边界、`validate_params` 各分支、各分组
- [ ] 构造 `UploadInputCtx` 时补 `work_dir` 字段
- [ ] `new_in_process` 路径指向 `target/<profile>/resources/<phase>/<name>`
- [ ] 注册时 `registry_config` 传扁平一层实际值（多形态记得带 `group`）
- [ ] 需要硬校验时用 `try_new` 而非 `new`
- [ ] `cargo test --workspace` 全绿
