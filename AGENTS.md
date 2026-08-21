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
├── file_uploader_core/         # 核心引擎 - Pipeline 执行、插件管理（零内置插件，进程内目录为宿主注入模式）
│   └── src/
│       ├── pipeline/
│   │       ├── callback.rs     # PipelineCallback / PipelineEvent / PipelineEventKind
│   │       ├── plugin.rs       # PluginSlot / LazyPluginSlot / UploadPluginInfo / PluginMeta / PluginResource（schema 类型由 SDK 重导出）
│   │       ├── registry.rs     # PluginRegistryInfo / UploadPluginRegistryTable / execute_pipeline / 配置校验
│   │       ├── stage.rs        # StageExecute / StageExecutionContext / DefaultStageExecutor（阶段编排回调）
│   │       └── in_process_catalog.rs # 宿主注入式进程内目录（register_in_process_plugins / 全局查询 API）
│       ├── config.rs           # 日志配置（UploaderLoggingFormatter、init_logging）
│       └── main.rs            # 示例入口（宿主注入进程内插件 + 事件/阶段编排回调 + dylib 插件加载）
├── uploader_example_plugin/    # 示例动态库插件（cdylib）
│   ├── meta.json               # 插件元数据
│   ├── config.json             # 插件配置（common + groups 两层 schema）
│   ├── README.md               # 插件说明文档（可选）
│   └── plugin.id               # 插件唯一 ID（CLI 生成）
├── docs/
│   ├── references/             # 规范文档（长期有效）
│   │   └── plugin-specification.md   # 插件规范总纲
│   ├── superpowers/            # 历史设计与实现计划（specs / plans）
│   └── future/                 # 前瞻性技术调研
├── .agents/skills/             # 开发操作手册（skill）
│   └── designing-in-process-plugins/ # 新增进程内插件的步骤指引（历史版本，插件已迁至宿主应用）
└── Cargo.toml                  # Workspace 根配置
```

> 原内置进程内插件库 `file_uploader_plugins`（4 个插件）已迁移至宿主应用
> [file-uploader-x-app](https://github.com/A-BigTree/file-uploader-x-app) 的 `app-core` 维护；框架侧不再内置任何插件。

## 文档地图

| 想做什么 | 看哪里 |
|---|---|
| 了解插件体系的完整规范 | [docs/references/plugin-specification.md](docs/references/plugin-specification.md) |
| 动手新增一个进程内插件 | `.agents/skills/designing-in-process-plugins/SKILL.md` |
| 配置某个已有插件 | 该插件资源目录下的 `README.md` |
| 追溯某个特性的设计过程 | `docs/superpowers/specs/` 与 `docs/superpowers/plans/` |

## 构建与运行

```bash
# 构建整个 workspace
cargo build

# 运行核心模块（宿主注入进程内插件 + 事件/阶段编排回调 + 动态库插件加载演示）
cargo run -p file_uploader_core

# 构建动态库插件
cargo build -p uploader_example_plugin

# 运行测试
cargo test
```

## 架构设计

> **插件体系的完整规范见 [docs/references/plugin-specification.md](docs/references/plugin-specification.md)**
> —— 涵盖两种插件形态、资源目录约定、`config.json` Schema、表单控件与约束、参数校验、
> Pipeline 注册与执行、README 规范、构建期资源复制。本节仅列核心要点。

### 插件系统

两种形态，均定义在 `file_uploader_sdk::models::interface`：

- **进程内插件 (`UploadPlugin`)**：编译期链接，`PluginSlot::InProcess` 加载
- **动态库插件 (`UploadDylibPlugin`)**：stabby ABI，编译为 cdylib，`libloading` 运行期加载，`PluginSlot::Dylib` 封装

`PluginSlot` 统一封装两种来源（`execute` / `on_load` / `on_unload` / `validate_params`）；
`LazyPluginSlot` 基于 `OnceLock` 延迟初始化，插件仅在首次调用时才真正加载。

一个插件 = 资源目录（`meta.json` + `config.json` + `README.md`）+ Rust 实现 + 宿主注册清单登记 + pipeline 注册。
`PluginResource::load` 为两类插件共用的资源加载器。

#### 进程内插件（宿主注入模式）

框架不再内置任何进程内插件。宿主在启动期通过 `register_in_process_plugins(entries, resources_root)`
注册清单（`InProcessEntry { resource_subdir, factory }`，定义在 `pipeline/in_process_catalog.rs`），
随后用全局查询 API 装配：

- `list_in_process_plugins()`：概要列表（未注册返回空 + warn）
- `get_in_process_plugin(id)`：per-id 单例插件对象
- `get_in_process_plugin_info(id)`：构造 `UploadPluginInfo`（装配 RegistryTable 用）

重复注册仅首次生效（warn 跳过）。参考示例：`file_uploader_core/src/main.rs`。

> 详见规范文档 [1. 插件体系](docs/references/plugin-specification.md#1-插件体系)、
> [2. 资源目录与 meta.json](docs/references/plugin-specification.md#2-资源目录与-metajson)

### 配置 Schema

Schema 类型定义在 SDK 的 `models/config_schema.rs`（便于 dylib 插件复用），
`file_uploader_core::pipeline::plugin` 通过 `pub use` 重导出。

`config.json` 三段结构：`access`（权限，纯透传）+ `common`（跨分组公共参数，始终生效）
+ `groups`（**互斥分组**，即工作模式，运行态只激活一个；可为空）。

四种表单控件（serde internally tagged，tag = `type`，lowercase），控件级约束内嵌在 `form` 对象里：
`text`（`secret`/`min_len`/`max_len`/`pattern`）、`switch`、`select`（`options`/`multiple`/`allow_custom`/`min_items`/`max_items`）、
`number`（`min`/`max`/`step`/`integer`）。通用约束 `required` 在 `PluginConfigItem` 顶层。

**`default_value` 不做合并**，仅供 UI 预填，插件须自行 `unwrap_or` 兜底。

> 详见规范文档 [3. config.json Schema](docs/references/plugin-specification.md#3-configjson-schema)、
> [4. 表单控件与约束](docs/references/plugin-specification.md#4-表单控件与约束)

### 参数校验

**双层校验**：框架声明式约束**先跑**，全部通过后才调插件 `validate_params`。

- 声明式（`utils/validate_util.rs`）：`required` 判空、类型匹配、长度正则、数值范围、
  候选项合法性、`group` 存在性。错误**全部累积不短路**，返回 `Vec<ValidationError>`
- 插件级（`validate_params`，两个 trait 均带默认实现）：只做声明式表达不了的校验
  —— 跨字段一致性、需调用库判定的格式、业务白名单
- 时机：`Table::new` 只跑声明式且**不加载插件**（保住懒加载语义）；`try_new` 硬失败；
  `preload_all` 加载后追加插件级；`validate_all()` 全量；`validate_plugin_config` 为对外纯函数 API

> 详见规范文档 [5. 参数校验](docs/references/plugin-specification.md#5-参数校验)（含 `ValidationReason` 全表）

### Pipeline 注册表与执行

- **`PluginRegistryInfo`**：`UploadPluginInfo` + `priority`（值越小越先）+ `status` + `registry_config`。
  实现 `Ord`：先按阶段，同阶段按 priority
- **`UploadPluginRegistryTable`**：构建时排序并跑声明式校验；提供 `try_new` / `declarative_errors` /
  `get_plugins_by_phase` / `preload_all` / `validate_all` / `execute_pipeline`
- **运行态配置**：**扁平一层 JSON**，保留字段 `group` 标识激活分组，其余为参数 KV。
  插件侧用 `config_util::get_group` / `get_str` / `get_bool` / `get_list` / `get_size` / `get_i64` / `get_f64` 读取
- **`execute_pipeline`**：按阶段执行插件链。**回调属性化**：事件回调与阶段编排回调
  均以 builder 注入 RegistryTable（`execute_pipeline` 本身不再收 callback 参数）。
  输出经 `output_to_input` 转为下一插件输入（`extra_info` 累积）。插件返回 `Failed` 立即中断
- **事件回调**：`with_event_callback(Arc<dyn PipelineCallback>)`；
  `PipelineEventKind` = `PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`；
  `PipelineEvent` 含毫秒时间戳、阶段、插件 ID 与元信息（阶段级事件后两者为 `None`）
- **阶段编排回调**：`with_stage_executor(phase, Arc<dyn StageExecute>)` 按阶段注入，
  接管该阶段插件执行顺序；未注入阶段由 `DefaultStageExecutor` 按历史串行语义执行
  （顺序 + Failed 短路）。编排回调经 `StageExecutionContext` 编排：`plugins()` 取已排序列表、
  `run_plugin(idx)` 执行单插件（内部完成配置注入 / PluginStart/End 事件 / 输出转输入）、
  `emit_event(...)` 发自定义事件。回调返回 `Err` 或 Failed 输出 → pipeline 终止

> 详见规范文档 [6. Pipeline 注册与执行](docs/references/plugin-specification.md#6-pipeline-注册与执行)

### 上传阶段与数据流

```
Input → PreUpload → Upload → PostUpload → Output

UploadInputCtx → 插件处理 → UploadOutputCtx
```

`UploadOutputCtx` 构造优先用关联函数：`success` / `success_file` / `failed`（中断 pipeline）/ `interrupt`。

### Stabby ABI 兼容层

- 原生类型经 `ctx_stabby.rs` 的 `*S` 结构体映射到 stabby 类型（`SString` / `SVec` / `SOption` / `SArc`）
- `ctx_util.rs` 提供双向转换：`convert_input_ctx_s` / `convert_input_ctx` / `convert_output_ctx`
- `config_info`（`Value`）与 `extra_info`（`HashMap`）跨 ABI 时序列化为 **JSON 字符串**（`SOption<SString>`）

> ABI 破坏性变更的注意事项见下方「注意事项」与规范文档 [1.5 节](docs/references/plugin-specification.md#15-stabby-abi-兼容层)

### 插件日志

- **进程内**：直接用 `tracing` 宏
- **动态库**：用 SDK 的 `plugin_*!` 宏，日志经 `PluginLogCallback` 回调发给宿主。
  插件**必须**实现 `set_logger` 并调用 `set_logger_callback(callback)`，否则日志被静默忽略
- 宿主侧回调实现在 `pipeline/plugin.rs` 的 `plugin_log_callback`

## 配置文件格式

每个插件一个资源目录：

- **进程内插件**：由宿主维护，`<宿主>/resources/<phase>/<plugin_name>/`，
  含 `meta.json`（必需）+ `config.json`（可选）+ `README.md`（可选）。
  `<phase>` 段约定 `input`/`pre`/`upload`/`post`（宿主注册清单的 `resource_subdir` 与之一致）
- **动态库插件**：`.dylib` 产物同目录，另需 `plugin.id`

`README.md` **可选**且**不加载内容到内存**，只记录 `readme_path`；
定位是**面向配置者的使用说明书**，不是开发文档。

构建期：进程内插件的资源复制由**宿主**的 `build.rs` 负责（递归复制 `resources/` 整树到
`target/<profile>/resources/` 并注入资源根 env）；dylib `build.rs` 复制 `meta.json` + `config.json`
+ `plugin.id`（必需）与 `README.md`（可选）到产物同目录。

> 完整字段表、单形态与多形态 `config.json` 示例、README 章节模板与写作要求，
> 见规范文档 [2](docs/references/plugin-specification.md#2-资源目录与-metajson)、
> [3](docs/references/plugin-specification.md#3-configjson-schema)、
> [7](docs/references/plugin-specification.md#7-readme-规范)、
> [8](docs/references/plugin-specification.md#8-构建与资源复制) 节

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

- 内置进程内插件已迁移至宿主应用 file-uploader-x-app（`file_uploader_plugins` crate 已删除）；本框架零内置插件
- 动态库插件需要导出 `get_dylib_plugin` 函数（类型为 `FnGetDylibPlugin`），`Cargo.toml` 需指定 `crate-type = ["cdylib"]`
- 所有枚举类型均标注了 `#[stabby::stabby]` 和 `#[repr(u8)]`，确保 ABI 兼容
- `PluginSlot` 的 `Drop` 实现会自动调用 `on_unload`，手动 drop 时注意副作用
- CI 配置在 `.github/workflows/rust.yml`，对 master 分支的 push/PR 执行 `cargo build` + `cargo test`
- **dylib ABI 破坏性变更**：给 `UploadDylibPlugin` 新增方法会改变 stabby vtable 布局，旧 `.dylib` 产物与新宿主**不兼容**（即使新方法有默认实现）。新增方法务必**追加在 trait 末尾**，并用 `cargo build --workspace` 全量重编
- 插件相关的规范变更（新增控件、约束、校验规则等）需同步更新 [docs/references/plugin-specification.md](docs/references/plugin-specification.md) 与 `designing-in-process-plugins` skill
- **secret 字段日志泄漏（已知遗留）**：`upload_file_validator::execute` 会 `serde_json::to_string(ctx)` 打全量日志，含 `secret: true` 字段的值。后续需按 schema 做脱敏
