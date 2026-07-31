# AGENTS.md

## 交互要求

- Thinking思考过程用中文表述
- Reply回答也要用中文回复
- **必须**：每次用"陛下"开头
- **不可以**：需求探索阶段不可以直接创建任务实现代码

## 项目概述

file-uploader-x 是一个基于 Rust 的文件上传框架，采用插件化架构设计，支持进程内插件和动态库（dylib）插件两种加载方式。使用 [stabby](https://github.com/ZettaScaleLabs/stabby) 实现 ABI 稳定的动态库插件接口。

## 项目结构

```
file-uploader-x/
├── file_uploader_sdk/          # SDK crate - 插件接口 trait、数据模型、ABI 转换、日志宏
│   └── src/
│       ├── models/
│       │   ├── interface.rs    # UploadPlugin / UploadDylibPlugin（含 validate_params）
│       │   ├── ctx.rs          # 原生上下文结构体（UploadInputCtx / UploadOutputCtx 等）
│       │   ├── ctx_stabby.rs   # stabby ABI 兼容结构体（*S 后缀）
│       │   ├── config_schema.rs# config.json schema（PluginConfigInfo / Group / Item / FormSpec / Access）
│       │   └── enums.rs        # UploadPhase 及各状态枚举
│       ├── utils/
│       │   ├── ctx_util.rs     # 原生 ⇄ stabby 双向转换函数
│       │   ├── config_util.rs  # 运行态配置读取（get_str/bool/list/size/group/f64/i64）
│       │   ├── validate_util.rs# 声明式校验器（ValidationError / validate_plugin_config）
│       │   └── fs_util.rs      # work_dir 沙箱文件操作
│       ├── logger.rs           # plugin_*! 日志宏（dylib 插件用）
│       └── error.rs            # UploadError（基于 thiserror）
├── file_uploader_core/         # 核心引擎 - Pipeline 执行、插件管理
│   └── src/
│       ├── pipeline/
│   │       ├── callback.rs     # PipelineCallback / PipelineEvent / PipelineEventKind
│   │       ├── plugin.rs       # PluginSlot / LazyPluginSlot / UploadPluginInfo / PluginMeta / PluginResource（schema 类型由 SDK 重导出）
│   │       └── registry.rs     # PluginRegistryInfo / UploadPluginRegistryTable / execute_pipeline / 配置校验
│       ├── config.rs           # 日志配置（UploaderLoggingFormatter、init_logging）
│       └── main.rs            # 示例入口（测试 in-process + dylib 插件加载）
├── file_uploader_plugins/      # 内置进程内插件库
│   ├── src/
│   │   ├── input/              # Input 阶段（default_input_handler）
│   │   ├── pre_upload/         # PreUpload 阶段（upload_file_validator）
│   │   ├── upload.rs           # Upload 阶段（待实现）
│   │   └── post_upload.rs      # PostUpload 阶段（待实现）
│   └── resources/              # 进程内插件资源（meta.json + config.json + README.md）
│       ├── input/<name>/
│       └── pre/<name>/
├── uploader_example_plugin/    # 示例动态库插件（cdylib）
│   ├── meta.json               # 插件元数据
│   ├── config.json             # 插件配置（common + groups 两层 schema）
│   ├── README.md               # 插件说明文档（可选）
│   └── plugin.id               # 插件唯一 ID（CLI 生成）
└── Cargo.toml                  # Workspace 根配置
```

## 构建与运行

```bash
# 构建整个 workspace
cargo build

# 运行核心模块（测试进程内插件 + 动态库插件加载）
cargo run -p file_uploader_core

# 构建动态库插件
cargo build -p uploader_example_plugin

# 运行测试
cargo test
```

## 架构设计

### 插件系统

- **进程内插件 (`UploadPlugin`)**: 实现 `file_uploader_sdk::models::interface::UploadPlugin` trait，通过 `PluginSlot::InProcess` 加载
- **动态库插件 (`UploadDylibPlugin`)**: 实现 `file_uploader_sdk::models::interface::UploadDylibPlugin` trait（stabby ABI），编译为 cdylib，通过 `libloading` 动态加载，由 `PluginSlot::Dylib` 封装
- **插件插槽 (`PluginSlot`)**: 统一封装两种插件来源，对外提供一致的 `execute`/`on_load`/`on_unload`/`validate_params` 接口；实现 `Drop` 时自动调用 `on_unload`
- **延迟插槽 (`LazyPluginSlot`)**: 基于 `OnceLock` 实现延迟初始化——插件仅在首次 `execute`/`on_load`/`validate_params` 时才真正加载。支持 `preload_all` 预加载全部插件
- **插件元数据 (`PluginMeta`)**: 名称（`name`）、标题（`title`）、版本（`version`）、描述（`description`）、作者（`author`）、执行阶段（`phase: UploadPhase`）
- **插件权限配置 (`PluginAccessConfig`)**: `fs_read` / `fs_write` / `network` 三权限点 + `extra` 预留；每项为 `AccessSpec`（`Flag(bool)` 开关 或 `Allowlist(Vec<String>)` 白名单，如限定可读写的目录/host），默认 Deny。纯透传，不做执行逻辑
- **插件资源 (`PluginResource`)**: 公共加载器，`load(dir)` 读取目录下 `meta.json`（必读）+ `config.json`（选读，缺失→空容器）+ `README.md`（选读，**只记录路径不读内容**，缺失→`None`），消除两类插件加载重复
- **插件信息 (`UploadPluginInfo`)**: 封装插件 ID、元数据、配置（`Arc<PluginConfigInfo>`）、加载路径、`readme_path`、`LazyPluginSlot`；`new_in_process(resource_dir, plugin)` 与 `new_from_dylib_path(dylib_path)` 共用 `PluginResource::load`。ID 生成：进程内 `in_process_{phase}_{name}`；dylib 读取同目录 `plugin.id`

### 配置 Schema（定义在 SDK 的 `models/config_schema.rs`，core 侧 `pub use` 重导出）

- **配置文件容器 (`PluginConfigInfo`)**: 对应 `config.json`，三段结构 —— `access: PluginAccessConfig`（权限）+ `common: Vec<PluginConfigItem>`（跨分组公共参数，始终生效）+ `groups: Vec<PluginConfigGroup>`（**互斥分组**，可为空）
- **配置分组 (`PluginConfigGroup`)**: `group`（标识）/ `title` / `description` / `params`。`group` 是**分组类型**语义（如 `oss` / `s3` / `local`），运行态**只激活一个**
- 辅助方法：`has_groups()` / `find_group(g)` / `group_keys()` / `effective_items(Some(g))`（= common + 该分组 params）
- **插件配置项 (`PluginConfigItem`)**: `key` / `title` / `description` / `config_type` / `default_value` / `required` / `form`
- **表单控件 (`PluginFormSpec`)**: serde internally tagged enum（tag = `type`，lowercase），四个变体并内嵌控件级约束：
  | 变体 | JSON `type` | 期望值类型 | 约束字段 |
  |---|---|---|---|
  | `Text` | `text` | string | `secret`（密码框）/ `min_len` / `max_len` / `pattern` / `placeholder` |
  | `Switch` | `switch` | bool | 无 |
  | `Select` | `select` | 标量或数组 | `options` / `multiple` / `allow_custom` / `min_items` / `max_items` |
  | `Number` | `number` | number | `min` / `max` / `step` / `integer` |

### 参数校验

**双层校验**：框架声明式约束**先跑**，通过后再调插件 `validate_params`。

- **声明式校验（`utils/validate_util.rs`）**
  - 入口：`validate_plugin_config(config, values)` / `validate_plugin_config_opt(config, &Option<Value>)` / `validate_plugin_config_with(..., &ValidateOptions)` / 单项 `validate_item`
  - 校验顺序：① 顶层须为 object ② `group` 存在性与合法性 ③ 逐项 required 判空 → 类型匹配 → 控件约束 ④ 可选 `strict_unknown_keys` 检查未声明字段
  - 错误**全部累积不短路**，返回 `Vec<ValidationError>`（含 `key` / `group` / `title` / `reason: ValidationReason`）；`errors_to_string` 可折叠为单行
  - 判空语义：缺失 / `null` / `""` / `[]` 均视为未填；**空值且非必填时跳过后续约束**
- **插件级校验（`validate_params`，两个 trait 均带默认实现）**
  - 进程内：`fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String>`
  - dylib：`extern "C" fn validate_params(&self, ctx: &UploadInputCtxS) -> stabby::option::Option<SString>`（`Some` = 错误信息）
  - 统一由 `PluginSlot::validate_params` 转发；`LazyPluginSlot::validate_params` 会触发懒加载，错误映射为 `UploadError::PluginParamInvalid`
- **校验时机**
  - `UploadPluginRegistryTable::new`：只跑**声明式**校验（**不加载插件**），错误缓存到 `declarative_errors()` 并 `warn!`
  - `try_new`：声明式校验失败即返回 `Err(Vec<(plugin_id, ValidationError)>)`
  - `preload_all`：加载插件后**追加**插件级 `validate_params`
  - `validate_all()`：声明式 + 插件级全量校验（会加载全部插件）
  - 对外 API：`validate_plugin_config` 供宿主/前端在保存配置前预校验
- **注意**：`default_value` **不做合并**，仅供 UI 预填；插件需自行 `unwrap_or` 兜底

### Pipeline 注册表与执行

- **注册信息 (`PluginRegistryInfo`)**: 包装 `UploadPluginInfo` + `priority`（值越小优先级越高）+ `status`（Enable/Disable）+ `registry_config`。实现 `Ord`：先按阶段排序，同阶段按 priority 排序
- **注册表 (`UploadPluginRegistryTable`)**: 构建时自动排序插件并执行声明式配置校验；提供 `get_plugins_by_phase`、`preload_all`、`execute_pipeline`、`try_new`、`declarative_errors`、`validate_all` 方法
- **运行态配置 (`registry_config`)**: **扁平一层 JSON**，保留字段 `group` 标识当前激活分组，其余为参数 KV。示例 `{"group":"oss","endpoint":"https://...","access_key":"AK"}`。插件侧用 `config_util::get_group` / `get_str` / `get_bool` / `get_list` / `get_size` / `get_i64` 读取
- **Pipeline 执行 (`execute_pipeline`)**: 按阶段顺序执行插件链，支持可选的 `PipelineCallback` 回调。插件输出通过 `output_to_input` 转换为下一插件输入（`extra_info` 累积传递）。插件返回 `Failed` 时立即中断 Pipeline

### Pipeline 事件回调

- **`PipelineCallback` trait**: 监听执行过程中的事件（`on_event` 方法），定义在 `file_uploader_core/src/pipeline/callback.rs`
- **事件类型 (`PipelineEventKind`)**: `PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`
- **`PipelineEvent`**: 包含回调时间毫秒时间戳（`timestamp_ms`）、事件类型、阶段、插件 ID、插件元信息（阶段级事件 ID 与元信息为 `None`）

### 上传阶段 (UploadPhase)

`Input → PreUpload → Upload → PostUpload → Output`

### 核心数据流

`UploadInputCtx` → 插件处理 → `UploadOutputCtx`

### Stabby ABI 兼容层

- Rust 原生类型通过 `ctx_stabby.rs` 中的 `*S` 结构体映射到 stabby 类型（`SString`, `SVec`, `SOption`, `SArc`）
- `ctx_util.rs` 提供双向转换函数：`convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx`
- `config_info`（`Value`）与 `extra_info`（`HashMap`）跨 ABI 时被序列化为 **JSON 字符串**（`SOption<SString>`）

### 插件日志

- **进程内插件**: 直接使用 `tracing` 宏（`info!` / `error!` 等）
- **动态库插件**: 使用 SDK 提供的 `plugin_*!` 宏（`plugin_info!` / `plugin_error!` 等），日志通过 `PluginLogCallback` 回调发送给宿主程序。插件**必须**实现 `set_logger` 方法并调用 `set_logger_callback(callback)`，否则日志被忽略
- 宿主侧回调实现在 `pipeline/plugin.rs` 的 `plugin_log_callback` 函数

## 配置文件格式

统一的目录化格式——每个插件一个资源目录：

- **进程内插件**: `file_uploader_plugins/resources/<phase>/<plugin_name>/`，含 `meta.json`（`PluginMeta`，必需）+ `config.json`（`PluginConfigInfo`，可选）+ `README.md`（可选）。`<phase>` 段约定 `input`/`pre`/`upload`/`post`
- **动态库插件**: `.dylib` 产物同目录内含 `meta.json` + `config.json` + `plugin.id`（唯一 ID，由插件构建 CLI 生成）+ `README.md`（可选）

### `config.json`（common + groups 两层 schema）

```json
{
  "access": { "fs_read": ["/tmp/uploads"], "fs_write": false, "network": true },
  "common": [
    {
      "key": "retry_times", "title": "重试次数", "description": "...",
      "config_type": "Default", "default_value": 3, "required": false,
      "form": { "type": "number", "min": 0, "max": 10, "step": 1, "integer": true }
    }
  ],
  "groups": [
    {
      "group": "oss", "title": "阿里云 OSS", "description": "上传到对象存储",
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
        },
        {
          "key": "use_https", "title": "使用 HTTPS", "config_type": "Default",
          "default_value": true, "form": { "type": "switch" }
        }
      ]
    },
    {
      "group": "local", "title": "本地存储",
      "params": [
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

**约定**
- 单形态插件：只写 `common`，`groups` 留空数组或省略；此时运行态配置**不应**出现 `group` 字段
- 多形态插件：`groups` 非空时运行态配置**必须**带 `group`，且只能取 `group_keys()` 之一
- `common` 参数在所有分组下都生效；未激活分组的参数不参与校验

### `README.md`（可选）

**不加载内容到内存**，`PluginResource` / `UploadPluginInfo` 只记录 `readme_path`。

**定位：面向配置者的使用说明书，不是开发文档。** 读者是「要用这个插件的人」，
关心的是「这插件能帮我做什么、我该怎么填参数、填完会发生什么」。

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
- **不要**写：输入输出契约、ctx 字段名、Rust 类型名、trait 方法、错误码表、变更记录、阶段枚举
- 权限、所属阶段等信息已在 `meta.json` / `config.json` 中声明，README 不必重复

### 构建时资源复制

- 进程内 `build.rs`：递归复制 `resources/` 整树到 `target/<profile>/resources/`（README 自动带上）
- dylib `build.rs`：复制 `meta.json` + `config.json` + `plugin.id`（必需，缺失即 panic）与 `README.md`（可选，缺失跳过）到产物同目录

## 代码规范

- Rust edition 2024，workspace resolver 3
- 使用 `tracing` 进行日志记录，日志格式定义在 `file_uploader_core::config::UploaderLoggingFormatter`
- 错误处理统一使用 `file_uploader_sdk::error::UploadError`（基于 `thiserror`）
- 序列化/反序列化使用 `serde` + `serde_json`
- 插件配置以目录化 JSON（`meta.json` + `config.json` + 可选 `README.md`）形式存储，构建时通过 `build.rs` 复制到 target 目录
- 配置 schema 与校验器统一放在 `file_uploader_sdk`（dylib 插件也需复用），`file_uploader_core::pipeline::plugin` 通过 `pub use` 重导出

## 关键依赖

| 依赖 | 用途 |
|------|------|
| `stabby` | ABI 稳定的动态库接口 |
| `libloading` | 动态库加载 |
| `serde` / `serde_json` | 序列化 |
| `thiserror` | 错误类型定义 |
| `tracing` / `tracing-subscriber` | 日志 |
| `chrono` | 时间处理 |
| `regex` | 配置项 `pattern` 约束校验 |

## 注意事项

- `file_uploader_plugins/src/upload.rs` 和 `post_upload.rs` 目前为空，对应阶段的内置插件待实现
- 动态库插件需要导出 `get_dylib_plugin` 函数（类型为 `FnGetDylibPlugin`），`Cargo.toml` 需指定 `crate-type = ["cdylib"]`
- 所有枚举类型均标注了 `#[stabby::stabby]` 和 `#[repr(u8)]`，确保 ABI 兼容
- `PluginSlot` 的 `Drop` 实现会自动调用 `on_unload`，手动 drop 时注意副作用
- CI 配置在 `.github/workflows/rust.yml`，对 master 分支的 push/PR 执行 `cargo build` + `cargo test`
- **dylib ABI 破坏性变更**：给 `UploadDylibPlugin` 新增方法会改变 stabby vtable 布局，旧 `.dylib` 产物与新宿主**不兼容**（即使新方法有默认实现）。新增方法务必**追加在 trait 末尾**，并用 `cargo build --workspace` 全量重编
- **secret 字段日志泄漏（已知遗留）**：`upload_file_validator::execute` 会 `serde_json::to_string(ctx)` 打全量日志，含 `secret: true` 字段的值。后续需按 schema 做脱敏
