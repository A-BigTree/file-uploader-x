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
├── file_uploader_sdk/     # SDK crate - 定义插件接口、数据模型、工具函数
├── file_uploader_core/    # 核心引擎 - 插件管理、Pipeline 执行、文件验证
├── uploader_example_plugin/ # 示例动态库插件（cdylib）
├── plugin.json            # 内置插件配置（file-type-filter）
└── Cargo.toml             # Workspace 根配置
```

## 构建与运行

```bash
# 构建整个 workspace
cargo build

# 运行核心模块（包含文件验证示例）
cargo run -p file_uploader_core

# 构建动态库插件
cargo build -p uploader_example_plugin
```

## 架构设计

### 插件系统

- **进程内插件 (`UploadPlugin`)**: 实现 `file_uploader_sdk::models::interface::UploadPlugin` trait，通过 `PluginSlot::InProcess` 加载
- **动态库插件 (`UploadDylibPlugin`)**: 实现 `file_uploader_sdk::models::interface::UploadDylibPlugin` trait（stabby ABI），编译为 cdylib，通过 `libloading` 动态加载，由 `PluginSlot::Dylib` 封装
- **插件插槽 (`PluginSlot`)**: 统一封装两种插件来源，对外提供一致的 `execute`/`on_load`/`on_unload` 接口
- **插件元数据 (`PluginMeta`)**: 名称、版本、描述、作者、执行阶段（`UploadPhase`）
- **插件配置 (`PluginConfig`)**: 每个 plugin 对应一个 JSON 配置文件，定义配置项及其默认值

### 上传阶段 (UploadPhase)

`Input → PreUpload → Upload → PostUpload → Output`

### 核心数据流

`UploadTaskCtx` → `UploadProcessCtx` → `UploadInputCtx` → 插件处理 → `UploadOutputCtx`

### Stabby ABI 兼容层

- Rust 原生类型通过 `ctx_stabby.rs` 中的 `*S` 结构体映射到 stabby 类型（`SString`, `SVec`, `SOption`, `SArc`）
- `ctx_util.rs` 提供双向转换函数：`convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx`

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

- `file_uploader_core/src/pipeline/registry.rs` 目前为空，插件注册机制待实现
- `main.rs` 中的文件验证逻辑为示例代码，尚未与插件 Pipeline 集成
- 动态库插件需要导出 `get_dylib_plugin` 函数（类型为 `FnGetDylibPlugin`）
- 所有枚举类型均标注了 `#[stabby::stabby]` 和 `#[repr(u8)]`，确保 ABI 兼容
