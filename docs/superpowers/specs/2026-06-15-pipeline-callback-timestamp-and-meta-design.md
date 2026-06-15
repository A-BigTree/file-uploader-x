# 回调消息新增时间戳与插件元信息设计

## 概述

为流水线统一回调消息 `PipelineEvent` 新增两个字段：

1. **回调时间毫秒时间戳** `timestamp_ms: i64` —— 在回调触发瞬间填充，便于回调方记录时序、计算插件执行耗时。
2. **插件元信息** `plugin_meta: Option<&'a PluginMeta>` —— 让回调方拿到插件全貌（名称/标题/版本/描述/作者/阶段），无需再自行反查。

同时，由于 `PluginMeta` 定义在 `file_uploader_core`，而 `PipelineEvent` 原先定义在 `file_uploader_sdk`（core 依赖 sdk，不可反向依赖），为消除 crate 依赖方向冲突，将**回调消息定义与回调接口整体上移到 `file_uploader_core`**。

## 动机

- 当前 `PipelineEvent` 仅有 `kind / phase / plugin_id`，回调方无法得知「何时触发」与「插件是什么」，需要额外查询，信息不内聚。
- 现有设计稿（`2026-06-12-pipeline-execute-with-callback-design.md`）曾设想 `plugin_meta` 字段，但实现时被简化丢弃，本次补齐。
- 把回调类型从 sdk 上移到 core，使其能直接复用 core 的 `PluginMeta`，消除依赖方向问题，也让回调类型归属于真正使用它的 pipeline 执行层。

## 类型迁移

将以下三个类型从 `file_uploader_sdk/src/models/interface.rs` **整体迁移**到 `file_uploader_core/src/pipeline/callback.rs`（新建文件）：

- `PipelineEventKind`
- `PipelineEvent<'a>`
- `PipelineCallback`

迁移后 sdk 中删除这三个类型的定义与相关导出。core 通过 `crate::pipeline::callback` 模块导出。

> 依赖说明：`chrono` 已在 `file_uploader_core/Cargo.toml`（workspace）中可用，无需新增依赖。

## 类型定义

定义在 `file_uploader_core/src/pipeline/callback.rs`：

```rust
use crate::pipeline::plugin::PluginMeta;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineEventKind {
    PhaseStart,
    PhaseEnd,
    PluginStart,
    PluginEnd,
}

pub struct PipelineEvent<'a> {
    /// 回调触发瞬间的毫秒时间戳（当地系统时间，i64 epoch 毫秒）
    pub timestamp_ms: i64,
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
    /// 插件元信息：插件级事件填充对应插件；阶段级事件为 None
    pub plugin_meta: Option<&'a PluginMeta>,
}

pub trait PipelineCallback: Send + Sync {
    fn on_event(
        &self,
        event: &PipelineEvent,
        ctx: &UploadInputCtx,
        result: Option<&UploadOutputCtx>,
    );
}
```

### 字段填充规则

| 事件类型 | `timestamp_ms` | `plugin_id` | `plugin_meta` | `result` |
|----------|----------------|-------------|---------------|----------|
| `PhaseStart` | 触发瞬间 | `None` | `None` | `None` |
| `PluginStart` | 触发瞬间 | `Some(插件id)` | `Some(插件元信息)` | `None` |
| `PluginEnd` | 触发瞬间 | `Some(插件id)` | `Some(插件元信息)` | `Some(输出)` |
| `PhaseEnd` | 触发瞬间 | `None` | `None` | `Some(阶段最后输出)` |

### 关于 `timestamp_ms` 的说明

- 取值方式：`chrono::Local::now().timestamp_millis()`。
- 技术注记：epoch 毫秒时间戳（i64）与时区无关，`Local::now().timestamp_millis()` 与 `Utc::now().timestamp_millis()` 数值完全相同；此处采用 `Local` 以贴合「当地系统时间」的语义表达。
- 若回调方需要插件执行耗时，可用同一插件 `PluginStart` 与 `PluginEnd` 两个事件的 `timestamp_ms` 相减得到。

### 关于 `plugin_meta` 的说明

- `PluginMeta` 仍保留其全部字段（`name / title / version / description / author / phase`），本次不做删改；其中 `phase` 与 `event.phase` 存在语义冗余，但保留单一数据源完整性。
- 采用引用 `Option<&'a PluginMeta>` 形式（方案 A）：零拷贝，生命周期绑定到 `execute_pipeline` 的 `&self`。`on_event` 为同步调用，引用方案无跨 `await`/线程持有问题。

## execute_pipeline 改造

`file_uploader_core/src/pipeline/registry.rs` 的 `execute_pipeline` 方法中，共 5 处回调触发代码点（1× PhaseStart、1× PluginStart、2× PluginEnd —— 正常与失败两个互斥分支各 1 处、1× PhaseEnd）。每处触发点在构造 `PipelineEvent` 之前：

```rust
let now_ms = chrono::Local::now().timestamp_millis();
```

并按填充规则填入 `timestamp_ms` 与 `plugin_meta`：

- 阶段级事件（PhaseStart / PhaseEnd）：
  ```rust
  let event = PipelineEvent {
      timestamp_ms: now_ms,
      kind: PipelineEventKind::PhaseStart, // 或 PhaseEnd
      phase: phase.clone(),
      plugin_id: None,
      plugin_meta: None,
  };
  ```
- 插件级事件（PluginStart / PluginEnd）：
  ```rust
  let event = PipelineEvent {
      timestamp_ms: now_ms,
      kind: PipelineEventKind::PluginStart, // 或 PluginEnd
      phase: phase.clone(),
      plugin_id: Some(&plugin.plugin_instance.id),
      plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
  };
  ```

`registry.rs` 顶部 import 由：

```rust
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
```

改为：

```rust
use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
```

## 文件变更清单

| 文件 | 变更内容 |
|------|----------|
| `file_uploader_core/src/pipeline/callback.rs` | **新建**：迁移并扩展 `PipelineEventKind`、`PipelineEvent<'a>`（新增 `timestamp_ms`、`plugin_meta`）、`PipelineCallback` |
| `file_uploader_core/src/pipeline.rs` | 新增 `pub mod callback;` |
| `file_uploader_sdk/src/models/interface.rs` | **删除** `PipelineEventKind`、`PipelineEvent`、`PipelineCallback` 及相关 `use`（第 52–75 行：`use UploadPhase` 及三个类型定义） |
| `file_uploader_core/src/pipeline/registry.rs` | import 改为 `crate::pipeline::callback::{...}`；6 个回调点填充 `timestamp_ms` 与 `plugin_meta` |
| `file_uploader_core/src/pipeline/registry.rs`（tests） | `CallbackRecord` 增加 `timestamp_ms`、`plugin_meta` 字段；`TestCallback::on_event` 记录新字段；新增断言 |
| `README.md` | 第 103 行附近 `PipelineCallback` 示例的 import 路径由 sdk 改为 core |
| `AGENTS.md` | 第 81–83 行回调类型位置说明更新为 core |

## 测试计划

在 `registry.rs` 既有测试基础上：

1. **既有测试不破坏**：`test_execute_pipeline_single_plugin_callback_order`、`test_execute_pipeline_plugin_failed_interrupts`、`test_execute_pipeline_registry_config_injected` 继续通过（更新 `CallbackRecord` 与 import 后）。
2. **时间戳断言**：所有记录的 `timestamp_ms > 0`；且事件序列中 `timestamp_ms` 单调非递减。
3. **元信息断言**：
   - 阶段级事件（PhaseStart / PhaseEnd）的 `plugin_meta == None`。
   - 插件级事件（PluginStart / PluginEnd）的 `plugin_meta == Some(...)`，且其 `name` 等于对应插件名（如 `"p1"`）。
4. **失败分支元信息**：插件执行失败时，失败分支的 `PluginEnd` 事件仍携带该插件的 `plugin_meta`。

## 影响面与兼容性

- **破坏性变更**：`PipelineEventKind` / `PipelineEvent` / `PipelineCallback` 从 `file_uploader_sdk` 移除，导入路径变更。
- **实际影响**：经全仓搜索，除 `file_uploader_core/src/pipeline/registry.rs`（含其 tests）外，无其他代码引用这些类型；`main.rs` 未使用回调。因此无外部破坏。
- `PluginMeta` 定义位置与字段不变，仅是被 `PipelineEvent` 以引用形式包含。
