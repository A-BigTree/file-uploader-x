# file-uploader-x

一个基于 Rust 的插件化文件上传框架。采用进程内插件与动态库（dylib）插件双模式架构，通过 [stabby](https://github.com/ZettaScaleLabs/stabby) 实现 ABI 稳定的跨编译器动态库插件接口。

## 特性

- **双模式插件系统** — 同时支持进程内插件（`UploadPlugin`）和动态库插件（`UploadDylibPlugin`），通过 `PluginSlot` 统一封装
- **ABI 稳定** — 动态库插件基于 stabby，无需与宿主程序使用相同 Rust 编译器版本
- **Pipeline 执行引擎** — 按阶段（`Input → PreUpload → Upload → PostUpload → Output`）和优先级排序执行插件链
- **延迟加载** — 插件在首次 `execute` 时才初始化（`LazyPluginSlot`），支持 `preload_all` 预加载
- **事件回调** — 通过 `PipelineCallback` 监听阶段/插件的开始与结束事件
- **插件配置** — 每个 JSON 配置文件定义插件元数据与可配置项

## 项目结构

```
file-uploader-x/
├── file_uploader_sdk/          # SDK - 插件接口 trait、数据模型、ABI 转换、日志宏
│   └── src/
│       ├── models/
│   │       ├── interface.rs    # UploadPlugin / UploadDylibPlugin
│   │       ├── ctx.rs          # 原生上下文结构体
│   │       ├── ctx_stabby.rs   # stabby ABI 兼容结构体（*S 后缀）
│   │       └── enums.rs        # UploadPhase 及各状态枚举
│       ├── utils/ctx_util.rs   # 原生 ⇄ stabby 双向转换
│       ├── logger.rs           # plugin_*! 日志宏（dylib 插件用）
│       └── error.rs            # UploadError
├── file_uploader_core/         # 核心引擎 - Pipeline 执行、插件管理
│   └── src/
│       ├── pipeline/
│   │       ├── callback.rs     # PipelineCallback / PipelineEvent / PipelineEventKind
│   │       ├── plugin.rs       # PluginSlot / LazyPluginSlot / UploadPluginInfo / PluginMeta
│   │       └── registry.rs     # UploadPluginRegistryTable / execute_pipeline
│       ├── config.rs           # 日志格式与初始化
│       └── main.rs            # 示例入口
├── file_uploader_plugins/      # 内置进程内插件库
│   └── src/
│       ├── pre_upload/         # PreUpload 阶段插件（file_type_filter）
│       ├── upload.rs           # Upload 阶段（待实现）
│       └── post_upload.rs      # PostUpload 阶段（待实现）
├── uploader_example_plugin/    # 示例动态库插件（cdylib）
└── Cargo.toml                  # Workspace 根配置
```

## 快速开始

### 环境要求

- Rust 工具链（edition 2024）

### 构建

```bash
# 构建整个 workspace（包含动态库插件）
cargo build

# 运行示例（测试进程内插件 + 动态库插件加载）
cargo run -p file_uploader_core
```

### 运行测试

```bash
cargo test
```

## 架构设计

### 上传阶段

插件按 5 个阶段顺序执行，同阶段内按 `priority`（值越小优先级越高）排序：

```
Input → PreUpload → Upload → PostUpload → Output
```

### 数据流

```
UploadTaskCtx → UploadProcessCtx → UploadInputCtx → [插件处理] → UploadOutputCtx
```

每个插件的输出会被转换为下一个插件的输入（`output_to_input`），`extra_info` 会累积传递。

### 插件类型

| 类型 | Trait | 编译产物 | 加载方式 |
|------|-------|---------|---------|
| 进程内插件 | `UploadPlugin` | 普通库 | `PluginSlot::InProcess` |
| 动态库插件 | `UploadDylibPlugin` | cdylib (`.dylib`/`.so`/`.dll`) | `PluginSlot::Dylib`（通过 `libloading`） |

两种插件均通过 `LazyPluginSlot` 实现**延迟初始化**——只有在首次调用 `execute` 或 `on_load` 时才会真正加载。

### Stabby ABI 兼容层

动态库插件通过 stabby 类型与宿主交换数据：

- Rust 原生类型 ↔ stabby 类型：`UploadInputCtx` ↔ `UploadInputCtxS`、`UploadFileData` ↔ `UploadFileDataS` 等
- 转换函数位于 `file_uploader_sdk::utils::ctx_util`：`convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx`
- 所有跨 ABI 边界的枚举标注 `#[stabby::stabby]` + `#[repr(u8)]`

### Pipeline 事件回调

实现 `PipelineCallback` trait 可监听执行过程中的事件：

`PipelineEvent` 包含回调时间毫秒时间戳（`timestamp_ms`）、事件类型、阶段、插件 ID 与插件元信息（`plugin_meta`，阶段级事件为 `None`）。

| 事件 | 触发时机 |
|------|---------|
| `PhaseStart` / `PhaseEnd` | 每个阶段开始 / 结束 |
| `PluginStart` / `PluginEnd` | 每个插件执行前 / 执行后 |

若插件返回 `Failed`，Pipeline 会立即中断并返回失败结果。

## 插件开发指南

### 开发进程内插件

```rust
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::UploadPlugin;

pub struct MyPlugin;

impl UploadPlugin for MyPlugin {
    fn name(&self) -> &'static str {
        "my-plugin"
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        // 处理逻辑...
        UploadOutputCtx {
            result: OutputResultType::Success,
            message: "done".to_string(),
            file_list: None,
            extra_info: None,
        }
    }

    fn on_load(&self) {
        // 插件加载时初始化
    }
}
```

配套的 JSON 配置文件（进程内插件按插件名作为 key 嵌套）：

```json
{
  "my-plugin": {
    "name": "my-plugin",
    "title": "我的插件",
    "description": "插件描述",
    "version": "0.0.1",
    "author": "Author",
    "phase": "PreUpload",
    "config": {
      "option_key": {
        "key": "option_key",
        "config_type": "Custom",
        "description": "配置说明",
        "default_value": "default"
      }
    }
  }
}
```

### 开发动态库插件

```rust
use file_uploader_sdk::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::{PluginLogCallback, UploadDylibPlugin};
use file_uploader_sdk::utils::ctx_util::convert_input_ctx;
use file_uploader_sdk::{logger::set_logger_callback, plugin_info};

pub struct MyDylibPlugin;

impl UploadDylibPlugin for MyDylibPlugin {
    extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS {
        plugin_info!("MyDylibPlugin executing...");
        let ctx = convert_input_ctx(ctx);
        // 处理逻辑...
        UploadOutputCtxS {
            result: OutputResultType::Success,
            message: "成功".to_string().into(),
            file_list: stabby::option::Option::None(),
            extra_info: stabby::option::Option::None(),
        }
    }

    extern "C" fn set_logger(&self, callback: PluginLogCallback) {
        set_logger_callback(callback);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_dylib_plugin()
-> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>) {
    stabby::boxed::Box::new(MyDylibPlugin).into()
}
```

动态库插件的 `Cargo.toml` 需指定 `crate-type = ["cdylib"]`，并导出 `get_dylib_plugin` 函数。

> **日志**：动态库插件必须实现 `set_logger` 方法并调用 `set_logger_callback(callback)`，否则 `plugin_*!` 宏输出的日志将被忽略。

## 使用 Pipeline

```rust
use file_uploader_core::pipeline::plugin::{UploadPluginInfo, PluginMeta};
use file_uploader_core::pipeline::registry::{
    UploadPluginRegistryTable, PluginRegistryInfo, PluginRegistryStatus,
};

// 1. 加载插件（进程内或动态库）
let plugin = UploadPluginInfo::new_in_process("config.json", Box::new(MyPlugin))?;
let dylib_plugin = UploadPluginInfo::new_from_dylib_path("./libmy_plugin.dylib")?;

// 2. 注册到 Registry Table
let registry = UploadPluginRegistryTable::new(
    "my_pipeline".to_string(),
    vec![
        PluginRegistryInfo::new(Arc::new(plugin), 1, PluginRegistryStatus::Enable, None),
        PluginRegistryInfo::new(Arc::new(dylib_plugin), 2, PluginRegistryStatus::Enable, None),
    ],
);

// 3. 可选：预加载所有插件
registry.preload_all()?;

// 4. 执行 Pipeline
let result = registry.execute_pipeline(input_ctx, None);
```

## 关键依赖

| 依赖 | 用途 |
|------|------|
| [stabby](https://github.com/ZettaScaleLabs/stabby) | ABI 稳定的动态库接口 |
| [libloading](https://github.com/nagisa/rust-libloading) | 动态库加载 |
| `serde` / `serde_json` | 序列化 |
| `thiserror` | 错误类型定义 |
| `tracing` / `tracing-subscriber` | 日志 |
| `chrono` | 时间处理 |

## License

[MIT](LICENSE)
