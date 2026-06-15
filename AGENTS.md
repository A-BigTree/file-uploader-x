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
│   │       ├── plugin.rs       # PluginSlot / LazyPluginSlot / UploadPluginInfo / PluginMeta / PluginConfig
│   │       └── registry.rs     # PluginRegistryInfo / UploadPluginRegistryTable / execute_pipeline
│       ├── config.rs           # 日志配置（UploaderLoggingFormatter、init_logging）
│       └── main.rs            # 示例入口（测试 in-process + dylib 插件加载）
├── file_uploader_plugins/      # 内置进程内插件库
│   └── src/
│       ├── pre_upload/         # PreUpload 阶段（file_type_filter）
│       ├── upload.rs           # Upload 阶段（待实现）
│       └── post_upload.rs      # PostUpload 阶段（待实现）
├── uploader_example_plugin/    # 示例动态库插件（cdylib）
├── plugin.json                 # 内置插件配置（file-type-filter）
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
- **插件配置 (`PluginConfig`)**: 每个 plugin 对应一个 JSON 配置项，包含 `key`、`config_type`、`description`、`default_value`
- **插件信息 (`UploadPluginInfo`)**: 封装插件 ID、元数据、默认配置、加载路径、`LazyPluginSlot`；提供 `new_in_process`（从配置+实例创建）和 `new_from_dylib_path`（从 .dylib 路径加载）两种构造方式

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

两种 JSON 配置格式：

- **进程内插件**（如 `pre_upload_plugins.json`）: 以插件名作为顶层 key 嵌套，对应 `UploadPluginInfo::new_in_process` 按插件 `name()` 查找
- **动态库插件**（如 `config.json`）: 单插件扁平结构，放在与 `.dylib` 同目录，对应 `UploadPluginInfo::new_from_dylib_path`

构建时通过各 crate 的 `build.rs` 将 JSON 配置复制到 target 目录。

## 代码规范

- Rust edition 2024，workspace resolver 3
- 使用 `tracing` 进行日志记录，日志格式定义在 `file_uploader_core::config::UploaderLoggingFormatter`
- 错误处理统一使用 `file_uploader_sdk::error::UploadError`（基于 `thiserror`）
- 序列化/反序列化使用 `serde` + `serde_json`
- 插件配置以 JSON 文件形式存储，构建时通过 `build.rs` 复制到 target 目录

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
