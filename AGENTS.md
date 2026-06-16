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
│   │   ├── interface.rs    # UploadPlugin / UploadDylibPlugin
│       │   ├── ctx.rs          # 原生上下文结构体（UploadTaskCtx 等）
│       │   ├── ctx_stabby.rs   # stabby ABI 兼容结构体（*S 后缀）
│       │   └── enums.rs        # UploadPhase 及各状态枚举
│       ├── utils/ctx_util.rs   # 原生 ⇄ stabby 双向转换函数
│       ├── logger.rs           # plugin_*! 日志宏（dylib 插件用）
│       └── error.rs            # UploadError（基于 thiserror）
├── file_uploader_core/         # 核心引擎 - Pipeline 执行、插件管理
│   └── src/
│       ├── pipeline/
│   │       ├── callback.rs     # PipelineCallback / PipelineEvent / PipelineEventKind
│   │       ├── plugin.rs       # PluginSlot / LazyPluginSlot / UploadPluginInfo / PluginMeta / PluginConfigInfo / PluginConfigItem / PluginFormSpec / PluginResource
│   │       └── registry.rs     # PluginRegistryInfo / UploadPluginRegistryTable / execute_pipeline
│       ├── config.rs           # 日志配置（UploaderLoggingFormatter、init_logging）
│       └── main.rs            # 示例入口（测试 in-process + dylib 插件加载）
├── file_uploader_plugins/      # 内置进程内插件库
│   ├── src/
│   │   ├── pre_upload/         # PreUpload 阶段（file_type_filter）
│   │   ├── upload.rs           # Upload 阶段（待实现）
│   │   └── post_upload.rs      # PostUpload 阶段（待实现）
│   └── resources/              # 进程内插件资源（meta.json + config.json）
│       └── pre/<name>/
├── uploader_example_plugin/    # 示例动态库插件（cdylib）
│   ├── meta.json               # 插件元数据
│   ├── config.json             # 插件配置（表单驱动 schema）
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
- **插件插槽 (`PluginSlot`)**: 统一封装两种插件来源，对外提供一致的 `execute`/`on_load`/`on_unload` 接口；实现 `Drop` 时自动调用 `on_unload`
- **延迟插槽 (`LazyPluginSlot`)**: 基于 `OnceLock` 实现延迟初始化——插件仅在首次 `execute` 或 `on_load` 时才真正加载。支持 `preload_all` 预加载全部插件
- **插件元数据 (`PluginMeta`)**: 名称（`name`）、标题（`title`）、版本（`version`）、描述（`description`）、作者（`author`）、执行阶段（`phase: UploadPhase`）
- **插件配置文件 (`PluginConfigInfo`)**: 对应 `config.json`，含 `access`（reserved）与 `params: Vec<PluginConfigItem>`
- **插件配置项 (`PluginConfigItem`)**: `key` / `title` / `description` / `config_type` / `default_value` / `form`，其中 `form: PluginFormSpec` 为表单控件描述（`Text{secret}` / `Select{options,multiple,allow_custom}`），采用 serde internally tagged enum（tag = `type`）
- **插件资源 (`PluginResource`)**: 公共加载器，`load(dir)` 读取目录下 `meta.json`（必读）+ `config.json`（选读，缺失→空容器），消除两类插件加载重复
- **插件信息 (`UploadPluginInfo`)**: 封装插件 ID、元数据、配置（`Arc<PluginConfigInfo>`）、加载路径、`LazyPluginSlot`；`new_in_process(resource_dir, plugin)` 与 `new_from_dylib_path(dylib_path)` 共用 `PluginResource::load`。ID 生成：进程内 `in_process_{phase}_{name}`；dylib 读取同目录 `plugin.id`

### Pipeline 注册表与执行

- **注册信息 (`PluginRegistryInfo`)**: 包装 `UploadPluginInfo` + `priority`（值越小优先级越高）+ `status`（Enable/Disable）+ `registry_config`。实现 `Ord`：先按阶段排序，同阶段按 priority 排序
- **注册表 (`UploadPluginRegistryTable`)**: 构建时自动排序插件；提供 `get_plugins_by_phase`、`preload_all`、`execute_pipeline` 方法
- **Pipeline 执行 (`execute_pipeline`)**: 按阶段顺序执行插件链，支持可选的 `PipelineCallback` 回调。插件输出通过 `output_to_input` 转换为下一插件输入（`extra_info` 累积传递）。插件返回 `Failed` 时立即中断 Pipeline

### Pipeline 事件回调

- **`PipelineCallback` trait**: 监听执行过程中的事件（`on_event` 方法），定义在 `file_uploader_core/src/pipeline/callback.rs`
- **事件类型 (`PipelineEventKind`)**: `PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`
- **`PipelineEvent`**: 包含回调时间毫秒时间戳（`timestamp_ms`）、事件类型、阶段、插件 ID、插件元信息（阶段级事件 ID 与元信息为 `None`）

### 上传阶段 (UploadPhase)

`Input → PreUpload → Upload → PostUpload → Output`

### 核心数据流

`UploadTaskCtx` → `UploadProcessCtx` → `UploadInputCtx` → 插件处理 → `UploadOutputCtx`

### Stabby ABI 兼容层

- Rust 原生类型通过 `ctx_stabby.rs` 中的 `*S` 结构体映射到 stabby 类型（`SString`, `SVec`, `SOption`, `SArc`）
- `ctx_util.rs` 提供双向转换函数：`convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx`

### 插件日志

- **进程内插件**: 直接使用 `tracing` 宏（`info!` / `error!` 等）
- **动态库插件**: 使用 SDK 提供的 `plugin_*!` 宏（`plugin_info!` / `plugin_error!` 等），日志通过 `PluginLogCallback` 回调发送给宿主程序。插件**必须**实现 `set_logger` 方法并调用 `set_logger_callback(callback)`，否则日志被忽略
- 宿主侧回调实现在 `pipeline/plugin.rs` 的 `plugin_log_callback` 函数

## 配置文件格式

统一的目录化格式——每个插件一个资源目录：

- **进程内插件**: `file_uploader_plugins/resources/<phase>/<plugin_name>/`，含 `meta.json`（`PluginMeta`）+ `config.json`（`PluginConfigInfo`）。`<phase>` 段约定 `pre`/`upload`/`post`
- **动态库插件**: `.dylib` 产物同目录内含 `meta.json` + `config.json` + `plugin.id`（唯一 ID，由插件构建 CLI 生成）

`config.json` 示例（表单驱动 schema）：

```json
{
  "access": {},
  "params": [
    {
      "key": "pass_type", "title": "允许类型", "description": "...",
      "config_type": "Custom", "default_value": [],
      "form": { "type": "select", "multiple": true, "allow_custom": true }
    }
  ]
}
```

构建时：进程内 `build.rs` 递归复制 `resources/` 整树到 `target/resources/`；dylib `build.rs` 复制 `meta.json` + `config.json` + `plugin.id` 三件到产物同目录。

## 代码规范

- Rust edition 2024，workspace resolver 3
- 使用 `tracing` 进行日志记录，日志格式定义在 `file_uploader_core::config::UploaderLoggingFormatter`
- 错误处理统一使用 `file_uploader_sdk::error::UploadError`（基于 `thiserror`）
- 序列化/反序列化使用 `serde` + `serde_json`
- 插件配置以目录化 JSON（`meta.json` + `config.json`）形式存储，构建时通过 `build.rs` 复制到 target 目录

## 关键依赖

| 依赖 | 用途 |
|------|------|
| `stabby` | ABI 稳定的动态库接口 |
| `libloading` | 动态库加载 |
| `serde` / `serde_json` | 序列化 |
| `thiserror` | 错误类型定义 |
| `tracing` / `tracing-subscriber` | 日志 |
| `chrono` | 时间处理 |

## 注意事项

- `file_uploader_plugins/src/upload.rs` 和 `post_upload.rs` 目前为空，对应阶段的内置插件待实现
- 动态库插件需要导出 `get_dylib_plugin` 函数（类型为 `FnGetDylibPlugin`），`Cargo.toml` 需指定 `crate-type = ["cdylib"]`
- 所有枚举类型均标注了 `#[stabby::stabby]` 和 `#[repr(u8)]`，确保 ABI 兼容
- `PluginSlot` 的 `Drop` 实现会自动调用 `on_unload`，手动 drop 时注意副作用
- CI 配置在 `.github/workflows/rust.yml`，对 master 分支的 push/PR 执行 `cargo build` + `cargo test`
