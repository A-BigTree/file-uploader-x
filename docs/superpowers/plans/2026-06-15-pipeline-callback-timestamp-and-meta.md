# 回调消息新增时间戳与插件元信息 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `PipelineEvent` 新增 `timestamp_ms`（i64 毫秒时间戳）与 `plugin_meta`（`Option<&PluginMeta>`）两个字段，并将回调类型从 `file_uploader_sdk` 上移到 `file_uploader_core`。

**Architecture:** 先做纯迁移（类型从 sdk 移到 core 新建的 `callback.rs`，保持行为不变），再以 TDD 方式分两步加入 `timestamp_ms` 与 `plugin_meta` 两个字段，最后更新文档。每步均可独立编译、独立测试、独立提交。

**Tech Stack:** Rust (edition 2024)、chrono（`Local::now().timestamp_millis()`）、既有 `cargo test`。

**Spec:** `docs/superpowers/specs/2026-06-15-pipeline-callback-timestamp-and-meta-design.md`

---

## 文件结构

| 文件 | 责任 |
|------|------|
| `file_uploader_core/src/pipeline/callback.rs` | **新建**：承载 `PipelineEventKind` / `PipelineEvent<'a>` / `PipelineCallback` |
| `file_uploader_core/src/pipeline.rs` | 模块声明，新增 `pub mod callback;` |
| `file_uploader_core/src/pipeline/registry.rs` | `execute_pipeline` 的 5 处回调点填充新字段；import 改为 core 内部；tests 更新 |
| `file_uploader_sdk/src/models/interface.rs` | 删除三个回调类型 |
| `README.md` | import 路径示例更新 |
| `AGENTS.md` | 回调类型位置说明更新 |

---

## Task 1: 迁移回调类型从 sdk 到 core（纯迁移，行为不变）

**Files:**
- Create: `file_uploader_core/src/pipeline/callback.rs`
- Modify: `file_uploader_core/src/pipeline.rs:1-2`
- Modify: `file_uploader_core/src/pipeline/registry.rs:5` 和 `:304-306`
- Modify: `file_uploader_sdk/src/models/interface.rs:52-75`

- [ ] **Step 1: 新建 `file_uploader_core/src/pipeline/callback.rs`**

写入以下内容（原样迁移，暂不加新字段；暂不 import `PluginMeta`，因未使用）：

```rust
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
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
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

- [ ] **Step 2: 在 `file_uploader_core/src/pipeline.rs` 注册模块**

将：
```rust
pub mod plugin;
pub mod registry;
```
改为：
```rust
pub mod callback;
pub mod plugin;
pub mod registry;
```

- [ ] **Step 3: 修改 `registry.rs` 顶部 import（第 5 行）**

将：
```rust
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
```
改为：
```rust
use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
```

- [ ] **Step 4: 修改 `registry.rs` tests 模块 import（第 304-306 行）**

将：
```rust
    use file_uploader_sdk::models::interface::{
        PipelineCallback, PipelineEvent, PipelineEventKind,
    };
```
改为：
```rust
    use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
```

- [ ] **Step 5: 从 sdk 删除回调类型**

删除 `file_uploader_sdk/src/models/interface.rs` 第 52–75 行，即从：

```rust
use crate::models::enums::UploadPhase;
```
（含该行及其后空行）一直到文件末尾的：

```rust
pub trait PipelineCallback: Send + Sync {
    fn on_event(
        &self,
        event: &PipelineEvent,
        ctx: &UploadInputCtx,
        result: Option<&UploadOutputCtx>,
    );
}
```

删除后，`interface.rs` 末尾应为 `PluginLogCallback` 类型定义（第 50 行附近）：

```rust
pub type PluginLogCallback = extern "C" fn(level: PluginLogLevel, message: SString);
```

- [ ] **Step 6: 编译并运行全部测试，验证迁移无行为变化**

Run: `cargo build`
Expected: 编译通过，无错误。

Run: `cargo test`
Expected: 所有测试通过（与迁移前一致），特别是 `test_execute_pipeline_single_plugin_callback_order`、`test_execute_pipeline_plugin_failed_interrupts`、`test_execute_pipeline_registry_config_injected`。

- [ ] **Step 7: 提交**

```bash
git add file_uploader_core/src/pipeline/callback.rs file_uploader_core/src/pipeline.rs file_uploader_core/src/pipeline/registry.rs file_uploader_sdk/src/models/interface.rs
git commit -m "refactor(core): move PipelineCallback types from sdk to core"
```

---

## Task 2: 为 PipelineEvent 新增 timestamp_ms 字段（TDD）

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs`（tests CallbackRecord + on_event + 断言；execute_pipeline 5 处回调点）
- Modify: `file_uploader_core/src/pipeline/callback.rs`（PipelineEvent 加字段）

- [ ] **Step 1: 先写失败测试 —— 扩展 CallbackRecord 与 TestCallback**

在 `registry.rs` 的 tests 模块中，将 `CallbackRecord`（原第 657-661 行）：

```rust
    struct CallbackRecord {
        pub kind: PipelineEventKind,
        pub phase: UploadPhase,
        pub plugin_id: Option<String>,
    }
```
改为：
```rust
    struct CallbackRecord {
        pub timestamp_ms: i64,
        pub kind: PipelineEventKind,
        pub phase: UploadPhase,
        pub plugin_id: Option<String>,
    }
```

将 `TestCallback` 的 `on_event`（原第 675-688 行）：

```rust
    impl PipelineCallback for TestCallback {
        fn on_event(
            &self,
            event: &PipelineEvent,
            _ctx: &UploadInputCtx,
            _result: Option<&UploadOutputCtx>,
        ) {
            self.records.lock().unwrap().push(CallbackRecord {
                kind: event.kind.clone(),
                phase: event.phase.clone(),
                plugin_id: event.plugin_id.map(|s| s.to_string()),
            });
        }
    }
```
改为：
```rust
    impl PipelineCallback for TestCallback {
        fn on_event(
            &self,
            event: &PipelineEvent,
            _ctx: &UploadInputCtx,
            _result: Option<&UploadOutputCtx>,
        ) {
            self.records.lock().unwrap().push(CallbackRecord {
                timestamp_ms: event.timestamp_ms,
                kind: event.kind.clone(),
                phase: event.phase.clone(),
                plugin_id: event.plugin_id.map(|s| s.to_string()),
            });
        }
    }
```

- [ ] **Step 2: 在 `test_execute_pipeline_single_plugin_callback_order` 末尾增加时间戳断言**

在该测试函数末尾（原第 805 行 `assert!(matches!(records[0].phase, UploadPhase::PreUpload));` 之后）追加：

```rust
        for r in records.iter() {
            assert!(r.timestamp_ms > 0, "timestamp_ms should be positive");
        }
```

- [ ] **Step 3: 运行测试，验证编译失败**

Run: `cargo test --package file_uploader_core`
Expected: 编译失败，错误形如 `no field 'timestamp_ms' on type 'PipelineEvent'<...>`。

- [ ] **Step 4: 给 PipelineEvent 加 timestamp_ms 字段**

在 `file_uploader_core/src/pipeline/callback.rs`，将：

```rust
pub struct PipelineEvent<'a> {
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
}
```
改为：
```rust
pub struct PipelineEvent<'a> {
    /// 回调触发瞬间的毫秒时间戳（当地系统时间，i64 epoch 毫秒）
    pub timestamp_ms: i64,
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
}
```

- [ ] **Step 5: 在 execute_pipeline 的 5 处回调点填充 timestamp_ms**

每处都在构造 `PipelineEvent` 之前插入一行取时间，并在结构体里加 `timestamp_ms` 字段。

(a) PhaseStart（原第 207-214 行）：
```rust
            if let Some(cb) = &callback {
                let now_ms = chrono::Local::now().timestamp_millis();
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseStart,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, None);
            }
```

(b) PluginStart（原第 226-233 行）：
```rust
                if let Some(cb) = &callback {
                    let now_ms = chrono::Local::now().timestamp_millis();
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginStart,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, None);
                }
```

(c) PluginEnd 失败分支（原第 244-251 行）：
```rust
                        if let Some(cb) = &callback {
                            let now_ms = chrono::Local::now().timestamp_millis();
                            let event = PipelineEvent {
                                timestamp_ms: now_ms,
                                kind: PipelineEventKind::PluginEnd,
                                phase: phase.clone(),
                                plugin_id: Some(&plugin.plugin_instance.id),
                            };
                            cb.on_event(&event, &plugin_input, Some(&fail_ctx));
                        }
```

(d) PluginEnd 正常分支（原第 256-263 行）：
```rust
                if let Some(cb) = &callback {
                    let now_ms = chrono::Local::now().timestamp_millis();
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginEnd,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, Some(&output));
                }
```

(e) PhaseEnd（原第 276-283 行）：
```rust
            if let Some(cb) = &callback {
                let now_ms = chrono::Local::now().timestamp_millis();
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseEnd,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, phase_last_output.as_ref());
            }
```

- [ ] **Step 6: 运行测试，验证通过**

Run: `cargo test --package file_uploader_core`
Expected: 所有测试通过，包含新增的 `timestamp_ms > 0` 断言。

- [ ] **Step 7: 提交**

```bash
git add file_uploader_core/src/pipeline/callback.rs file_uploader_core/src/pipeline/registry.rs
git commit -m "feat(core): add timestamp_ms to PipelineEvent"
```

---

## Task 3: 为 PipelineEvent 新增 plugin_meta 字段（TDD）

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs`（tests CallbackRecord + on_event + 断言；execute_pipeline 5 处回调点）
- Modify: `file_uploader_core/src/pipeline/callback.rs`（import PluginMeta + PipelineEvent 加字段）

- [ ] **Step 1: 先写失败测试 —— 扩展 CallbackRecord 与 TestCallback**

在 `registry.rs` 的 tests 模块中，将 `CallbackRecord`：

```rust
    struct CallbackRecord {
        pub timestamp_ms: i64,
        pub kind: PipelineEventKind,
        pub phase: UploadPhase,
        pub plugin_id: Option<String>,
    }
```
改为：
```rust
    struct CallbackRecord {
        pub timestamp_ms: i64,
        pub kind: PipelineEventKind,
        pub phase: UploadPhase,
        pub plugin_id: Option<String>,
        pub plugin_meta_name: Option<String>,
    }
```

将 `TestCallback` 的 `on_event`：

```rust
            self.records.lock().unwrap().push(CallbackRecord {
                timestamp_ms: event.timestamp_ms,
                kind: event.kind.clone(),
                phase: event.phase.clone(),
                plugin_id: event.plugin_id.map(|s| s.to_string()),
            });
```
改为：
```rust
            self.records.lock().unwrap().push(CallbackRecord {
                timestamp_ms: event.timestamp_ms,
                kind: event.kind.clone(),
                phase: event.phase.clone(),
                plugin_id: event.plugin_id.map(|s| s.to_string()),
                plugin_meta_name: event.plugin_meta.map(|m| m.name.clone()),
            });
```

- [ ] **Step 2: 在回调测试中增加元信息断言（成功 + 失败分支）**

(2a) 在 `test_execute_pipeline_single_plugin_callback_order` 末尾（Task 2 Step 2 追加的时间戳断言之后）追加：

```rust
        // 阶段级事件无插件元信息
        assert!(matches!(records[0].kind, PipelineEventKind::PhaseStart));
        assert!(records[0].plugin_meta_name.is_none());
        assert!(matches!(records[3].kind, PipelineEventKind::PhaseEnd));
        assert!(records[3].plugin_meta_name.is_none());
        // 插件级事件携带插件元信息
        assert!(matches!(records[1].kind, PipelineEventKind::PluginStart));
        assert_eq!(records[1].plugin_meta_name.as_deref(), Some("p1"));
        assert!(matches!(records[2].kind, PipelineEventKind::PluginEnd));
        assert_eq!(records[2].plugin_meta_name.as_deref(), Some("p1"));
```

(2b) 在 `test_execute_pipeline_plugin_failed_interrupts` 末尾（原第 859 行 `assert_eq!(plugin_ends.len(), 2);` 之后）追加，验证失败分支的 PluginEnd 仍携带该插件（p2）的元信息：

```rust
        // 失败分支的 PluginEnd 仍携带失败插件(p2)的元信息
        assert_eq!(
            plugin_ends[1].plugin_meta_name.as_deref(),
            Some("p2"),
            "failed plugin PluginEnd should carry its plugin_meta"
        );
```

- [ ] **Step 3: 运行测试，验证编译失败**

Run: `cargo test --package file_uploader_core`
Expected: 编译失败，错误形如 `no field 'plugin_meta' on type 'PipelineEvent'<...>`。

- [ ] **Step 4: 给 callback.rs 加 PluginMeta import 与 plugin_meta 字段**

在 `file_uploader_core/src/pipeline/callback.rs` 顶部 import 区，将：

```rust
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;
```
改为：
```rust
use crate::pipeline::plugin::PluginMeta;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;
```

将 `PipelineEvent`：

```rust
pub struct PipelineEvent<'a> {
    /// 回调触发瞬间的毫秒时间戳（当地系统时间，i64 epoch 毫秒）
    pub timestamp_ms: i64,
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
}
```
改为：
```rust
pub struct PipelineEvent<'a> {
    /// 回调触发瞬间的毫秒时间戳（当地系统时间，i64 epoch 毫秒）
    pub timestamp_ms: i64,
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
    /// 插件元信息：插件级事件填充对应插件；阶段级事件为 None
    pub plugin_meta: Option<&'a PluginMeta>,
}
```

- [ ] **Step 5: 在 execute_pipeline 的 5 处回调点填充 plugin_meta**

在 Task 2 已加入 `timestamp_ms` 的基础上，每处再加 `plugin_meta` 字段。

(a) PhaseStart —— `plugin_meta: None`：
```rust
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseStart,
                    phase: phase.clone(),
                    plugin_id: None,
                    plugin_meta: None,
                };
```

(b) PluginStart —— `plugin_meta: Some(...)`：
```rust
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginStart,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                        plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
                    };
```

(c) PluginEnd 失败分支 —— `plugin_meta: Some(...)`：
```rust
                            let event = PipelineEvent {
                                timestamp_ms: now_ms,
                                kind: PipelineEventKind::PluginEnd,
                                phase: phase.clone(),
                                plugin_id: Some(&plugin.plugin_instance.id),
                                plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
                            };
```

(d) PluginEnd 正常分支 —— `plugin_meta: Some(...)`：
```rust
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginEnd,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                        plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
                    };
```

(e) PhaseEnd —— `plugin_meta: None`：
```rust
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseEnd,
                    phase: phase.clone(),
                    plugin_id: None,
                    plugin_meta: None,
                };
```

- [ ] **Step 6: 运行测试，验证通过**

Run: `cargo test --package file_uploader_core`
Expected: 所有测试通过，包含新增的 plugin_meta 断言。

- [ ] **Step 7: 运行全量构建与测试，确认无回归**

Run: `cargo build && cargo test`
Expected: workspace 全部编译通过，全部测试通过。

- [ ] **Step 8: 提交**

```bash
git add file_uploader_core/src/pipeline/callback.rs file_uploader_core/src/pipeline/registry.rs
git commit -m "feat(core): add plugin_meta to PipelineEvent"
```

---

## Task 4: 更新 README 与 AGENTS.md 文档

**Files:**
- Modify: `README.md`（第 103 行附近）
- Modify: `AGENTS.md`（第 81-83 行）

- [ ] **Step 1: 更新 README.md 的 PipelineCallback 示例 import 路径**

将 `README.md` 中出现的：

```rust
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
```
改为：
```rust
use file_uploader_core::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
```

> 若 README 中该 import 形态略有不同（如分行），以实际文本为准，仅替换路径部分 `file_uploader_sdk::models::interface` → `file_uploader_core::pipeline::callback`。执行时先读取 README 第 95–120 行确认确切文本再替换。

- [ ] **Step 2: 更新 AGENTS.md 第 81-83 行回调类型位置说明**

将：
```
- **`PipelineCallback` trait**: 监听执行过程中的事件（`on_event` 方法）
- **事件类型 (`PipelineEventKind`)**: `PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`
- **`PipelineEvent`**: 包含事件类型、阶段、插件 ID（阶段级事件为 `None`）
```
改为：
```
- **`PipelineCallback` trait**: 监听执行过程中的事件（`on_event` 方法），定义在 `file_uploader_core/src/pipeline/callback.rs`
- **事件类型 (`PipelineEventKind`)**: `PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`
- **`PipelineEvent`**: 包含回调时间毫秒时间戳（`timestamp_ms`）、事件类型、阶段、插件 ID、插件元信息（阶段级事件 ID 与元信息为 `None`）
```

> 另：AGENTS.md 第 21 行的目录树注释 `interface.rs # UploadPlugin / UploadDylibPlugin / PipelineCallback` 中 `PipelineCallback` 已不再位于 interface.rs，执行时读取该行，将 ` / PipelineCallback` 删除（interface.rs 现仅含 `UploadPlugin / UploadDylibPlugin`）。同时在目录树中 `pipeline/` 段落补充 `callback.rs` 条目（紧跟 `plugin.rs` 之后），描述为 `# PipelineCallback / PipelineEvent / PipelineEventKind`。

- [ ] **Step 3: 提交**

```bash
git add README.md AGENTS.md
git commit -m "docs: update callback type location in README and AGENTS.md"
```

---

## 完成标准

- `cargo build && cargo test` 全绿。
- `PipelineEvent` 含 `timestamp_ms: i64` 与 `plugin_meta: Option<&'a PluginMeta>`。
- 回调类型仅存在于 `file_uploader_core`，sdk 中已无残留。
- README 与 AGENTS.md 的路径/位置说明已同步。
