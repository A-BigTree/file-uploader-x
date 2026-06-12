# Pipeline Execute with Callback 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `UploadPluginRegistryTable` 新增 `execute_pipeline` 方法，按阶段顺序链式执行插件，支持阶段级+插件级回调，失败即中断。

**Architecture:** 定义 `PipelineCallback` trait（单方法 `on_event` + `PipelineEvent` 枚举），在 `UploadPluginRegistryTable::execute_pipeline` 中遍历 5 个 UploadPhase，每个阶段内按 priority 顺序执行插件，上一插件 OutputCtx 转为下一插件 InputCtx，`config_info` 取自 `registry_config`，`extra_info` 从 output 合并更新，失败即中断。

**Tech Stack:** Rust, file_uploader_sdk, file_uploader_core

---

## File Structure

| 文件 | 操作 | 职责 |
|------|------|------|
| `file_uploader_sdk/src/models/interface.rs` | 修改 | 新增 `PipelineEventKind`、`PipelineEvent<'a>`、`PipelineCallback` trait |
| `file_uploader_core/src/pipeline/registry.rs` | 修改 | 新增 `execute_pipeline` 方法 + `output_to_input` 辅助函数 + 测试 |

---

### Task 1: 新增 PipelineCallback 相关类型到 SDK

**Files:**
- Modify: `file_uploader_sdk/src/models/interface.rs`

- [ ] **Step 1: 在 interface.rs 末尾新增类型**

在 `file_uploader_sdk/src/models/interface.rs` 文件末尾（第 50 行之后）追加：

```rust
use crate::models::enums::UploadPhase;

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

- [ ] **Step 2: 编译验证**

Run: `cargo build -p file_uploader_sdk`
Expected: BUILD SUCCEEDED

- [ ] **Step 3: Commit**

```bash
git add file_uploader_sdk/src/models/interface.rs
git commit -m "feat(sdk): add PipelineEventKind, PipelineEvent, PipelineCallback trait"
```

---

### Task 2: 实现 execute_pipeline 方法

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs`

- [ ] **Step 1: 新增 use 声明**

在 `file_uploader_core/src/pipeline/registry.rs` 文件顶部（第 1-6 行之后）新增：

```rust
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
use std::collections::HashMap;
```

- [ ] **Step 2: 在 UploadPluginRegistryTable impl 块中新增 output_to_input 和 execute_pipeline**

在 `preload_all` 方法（第 159 行）之后、impl 块闭合 `}` 之前新增：

```rust
    fn output_to_input(
        output: &UploadOutputCtx,
        source_ctx: &UploadInputCtx,
    ) -> UploadInputCtx {
        let mut extra_info = source_ctx.extra_info.clone().unwrap_or_default();
        if let Some(ref output_extra) = output.extra_info {
            for (k, v) in output_extra {
                extra_info.insert(k.clone(), v.clone());
            }
        }

        UploadInputCtx {
            file_list: output.file_list.clone().unwrap_or_default(),
            config_info: Arc::new(None),
            extra_info: if extra_info.is_empty() {
                None
            } else {
                Some(extra_info)
            },
            related_process_info: source_ctx.related_process_info.clone(),
        }
    }

    pub fn execute_pipeline(
        &self,
        input_ctx: UploadInputCtx,
        callback: Option<&dyn PipelineCallback>,
    ) -> UploadOutputCtx {
        let phases = [
            UploadPhase::Input,
            UploadPhase::PreUpload,
            UploadPhase::Upload,
            UploadPhase::PostUpload,
            UploadPhase::Output,
        ];

        let mut current_ctx = input_ctx;
        let mut last_output: Option<UploadOutputCtx> = None;

        for phase in &phases {
            let phase_plugins = self.get_plugins_by_phase(phase.clone());
            if phase_plugins.is_empty() {
                continue;
            }

            if let Some(cb) = &callback {
                let event = PipelineEvent {
                    kind: PipelineEventKind::PhaseStart,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, None);
            }

            let mut phase_last_output: Option<UploadOutputCtx> = None;

            for plugin in &phase_plugins {
                let plugin_input = UploadInputCtx {
                    file_list: current_ctx.file_list.clone(),
                    config_info: Arc::new(plugin.registry_config.clone()),
                    extra_info: current_ctx.extra_info.clone(),
                    related_process_info: current_ctx.related_process_info.clone(),
                };

                if let Some(cb) = &callback {
                    let event = PipelineEvent {
                        kind: PipelineEventKind::PluginStart,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, None);
                }

                let output = match plugin.execute(&plugin_input) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        let fail_ctx = UploadOutputCtx {
                            result: file_uploader_sdk::models::enums::OutputResultType::Failed,
                            message: e.to_string(),
                            file_list: None,
                            extra_info: None,
                        };
                        if let Some(cb) = &callback {
                            let event = PipelineEvent {
                                kind: PipelineEventKind::PluginEnd,
                                phase: phase.clone(),
                                plugin_id: Some(&plugin.plugin_instance.id),
                            };
                            cb.on_event(&event, &plugin_input, Some(&fail_ctx));
                        }
                        return fail_ctx;
                    }
                };

                if let Some(cb) = &callback {
                    let event = PipelineEvent {
                        kind: PipelineEventKind::PluginEnd,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, Some(&output));
                }

                if matches!(
                    output.result,
                    file_uploader_sdk::models::enums::OutputResultType::Failed
                ) {
                    return output;
                }

                phase_last_output = Some(output.clone());
                current_ctx = Self::output_to_input(&output, &current_ctx);
            }

            if let Some(cb) = &callback {
                let event = PipelineEvent {
                    kind: PipelineEventKind::PhaseEnd,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, phase_last_output.as_ref());
            }

            last_output = phase_last_output;
        }

        match last_output {
            Some(output) => output,
            None => UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: String::new(),
                file_list: Some(current_ctx.file_list),
                extra_info: current_ctx.extra_info,
            },
        }
    }
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p file_uploader_core`
Expected: BUILD SUCCEEDED

- [ ] **Step 4: Commit**

```bash
git add file_uploader_core/src/pipeline/registry.rs
git commit -m "feat(core): add execute_pipeline method with callback support"
```

---

### Task 3: 编写 execute_pipeline 单元测试

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs` (tests 模块)

- [ ] **Step 1: 新增测试 import 和辅助结构**

在 `registry.rs` 的 `#[cfg(test)] mod tests` 块中，在现有 import（第 164-166 行）之后追加：

```rust
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
use std::sync::Mutex;
```

在 `phase_order` 函数之后（第 222 行之后）新增回调辅助结构：

```rust
struct CallbackRecord {
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<String>,
}

struct TestCallback {
    pub records: Mutex<Vec<CallbackRecord>>,
}

impl TestCallback {
    fn new() -> Self {
        TestCallback {
            records: Mutex::new(Vec::new()),
        }
    }
}

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

struct FailPlugin;

impl file_uploader_sdk::models::interface::UploadPlugin for FailPlugin {
    fn name(&self) -> &'static str {
        "fail_plugin"
    }

    fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
        UploadOutputCtx {
            result: file_uploader_sdk::models::enums::OutputResultType::Failed,
            message: "intentional failure".to_string(),
            file_list: None,
            extra_info: None,
        }
    }
}

fn create_fail_plugin_info(name: &str, phase: UploadPhase) -> Arc<UploadPluginInfo> {
    let meta = Arc::new(PluginMeta {
        name: name.to_string(),
        title: "Fail Plugin".to_string(),
        version: "1.0.0".to_string(),
        description: "Fail plugin".to_string(),
        author: Some("test".to_string()),
        phase,
    });
    let plugin = std::sync::Arc::new(FailPlugin)
        as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>;
    let slot = LazyPluginSlot {
        source: LazySlotSource::InProcess {
            config_path: "/test/path".to_string(),
            plugin,
        },
        inner: OnceLock::new(),
    };
    Arc::new(UploadPluginInfo {
        id: format!("test_{}", name),
        meta,
        default_config: None,
        path: "/test/path".to_string(),
        slot,
    })
}

struct ConfigReadPlugin {
    captured_config: Arc<Mutex<Arc<Option<Value>>>>,
}

impl ConfigReadPlugin {
    fn new() -> Self {
        ConfigReadPlugin {
            captured_config: Arc::new(Mutex::new(Arc::new(None))),
        }
    }
}

impl file_uploader_sdk::models::interface::UploadPlugin for ConfigReadPlugin {
    fn name(&self) -> &'static str {
        "config_read_plugin"
    }

    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
        *self.captured_config.lock().unwrap() = ctx.config_info.clone();
        UploadOutputCtx {
            result: file_uploader_sdk::models::enums::OutputResultType::Success,
            message: "ok".to_string(),
            file_list: None,
            extra_info: None,
        }
    }
}
```

- [ ] **Step 2: 测试 — 空注册表执行**

在最后一个现有测试函数之后追加：

```rust
#[test]
fn test_execute_pipeline_empty_registry() {
    let registry = UploadPluginRegistryTable::new("test".to_string(), vec![]);
    let input = UploadInputCtx {
        file_list: vec![],
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
    };
    let result = registry.execute_pipeline(input, None);
    assert!(matches!(
        result.result,
        file_uploader_sdk::models::enums::OutputResultType::Success
    ));
}
```

Run: `cargo test -p file_uploader_core test_execute_pipeline_empty_registry`
Expected: PASS

- [ ] **Step 3: 测试 — 单阶段单插件 + 回调触发顺序**

```rust
#[test]
fn test_execute_pipeline_single_plugin_callback_order() {
    let plugin_info = create_mock_plugin_info("p1", UploadPhase::PreUpload);
    let reg_info = PluginRegistryInfo::new(
        plugin_info,
        1,
        PluginRegistryStatus::Enable,
        None,
    );
    let registry =
        UploadPluginRegistryTable::new("test".to_string(), vec![reg_info]);

    let input = UploadInputCtx {
        file_list: vec![],
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
    };

    let cb = TestCallback::new();
    let result = registry.execute_pipeline(input, Some(&cb));
    assert!(matches!(
        result.result,
        file_uploader_sdk::models::enums::OutputResultType::Success
    ));

    let records = cb.records.lock().unwrap();
    assert_eq!(records.len(), 4);
    assert!(matches!(records[0].kind, PipelineEventKind::PhaseStart));
    assert!(matches!(records[1].kind, PipelineEventKind::PluginStart));
    assert!(matches!(records[2].kind, PipelineEventKind::PluginEnd));
    assert!(matches!(records[3].kind, PipelineEventKind::PhaseEnd));
    assert_eq!(records[1].plugin_id.as_deref(), Some("test_p1"));
    assert!(matches!(records[0].phase, UploadPhase::PreUpload));
}
```

Run: `cargo test -p file_uploader_core test_execute_pipeline_single_plugin_callback_order`
Expected: PASS

- [ ] **Step 4: 测试 — 插件执行失败中断**

```rust
#[test]
fn test_execute_pipeline_plugin_failed_interrupts() {
    let p1 = PluginRegistryInfo::new(
        create_mock_plugin_info("p1", UploadPhase::PreUpload),
        1,
        PluginRegistryStatus::Enable,
        None,
    );
    let p2 = PluginRegistryInfo::new(
        create_fail_plugin_info("p2", UploadPhase::Upload),
        1,
        PluginRegistryStatus::Enable,
        None,
    );
    let p3 = PluginRegistryInfo::new(
        create_mock_plugin_info("p3", UploadPhase::PostUpload),
        1,
        PluginRegistryStatus::Enable,
        None,
    );
    let registry =
        UploadPluginRegistryTable::new("test".to_string(), vec![p1, p2, p3]);

    let input = UploadInputCtx {
        file_list: vec![],
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
    };

    let cb = TestCallback::new();
    let result = registry.execute_pipeline(input, Some(&cb));

    assert!(matches!(
        result.result,
        file_uploader_sdk::models::enums::OutputResultType::Failed
    ));
    assert_eq!(result.message, "intentional failure");

    let records = cb.records.lock().unwrap();
    let phase_ends: Vec<_> = records
        .iter()
        .filter(|r| matches!(r.kind, PipelineEventKind::PhaseEnd))
        .collect();
    assert_eq!(phase_ends.len(), 1);
    assert!(matches!(phase_ends[0].phase, UploadPhase::PreUpload));

    let plugin_ends: Vec<_> = records
        .iter()
        .filter(|r| matches!(r.kind, PipelineEventKind::PluginEnd))
        .collect();
    assert_eq!(plugin_ends.len(), 2);
}
```

Run: `cargo test -p file_uploader_core test_execute_pipeline_plugin_failed_interrupts`
Expected: PASS

- [ ] **Step 5: 测试 — registry_config 注入到 config_info**

```rust
#[test]
fn test_execute_pipeline_registry_config_injected() {
    let captured = Arc::new(Mutex::new(Arc::new(None)));
    let plugin = Arc::new(ConfigReadPlugin {
        captured_config: captured.clone(),
    });
    let meta = Arc::new(PluginMeta {
        name: "config_read".to_string(),
        title: "Config Read".to_string(),
        version: "1.0.0".to_string(),
        description: "reads config".to_string(),
        author: None,
        phase: UploadPhase::Upload,
    });
    let slot = LazyPluginSlot {
        source: LazySlotSource::InProcess {
            config_path: "/test".to_string(),
            plugin: plugin as Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
        },
        inner: OnceLock::new(),
    };
    let plugin_info = Arc::new(UploadPluginInfo {
        id: "test_config_read".to_string(),
        meta,
        default_config: None,
        path: "/test".to_string(),
        slot,
    });

    let config_value = serde_json::json!({"key": "value"});
    let reg_info = PluginRegistryInfo::new(
        plugin_info,
        1,
        PluginRegistryStatus::Enable,
        Some(config_value.clone()),
    );
    let registry =
        UploadPluginRegistryTable::new("test".to_string(), vec![reg_info]);

    let input = UploadInputCtx {
        file_list: vec![],
        config_info: Arc::new(None),
        extra_info: None,
        related_process_info: None,
    };

    registry.execute_pipeline(input, None);

    let config = captured.lock().unwrap();
    assert_eq!(**config, Some(config_value));
}
```

Run: `cargo test -p file_uploader_core test_execute_pipeline_registry_config_injected`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add file_uploader_core/src/pipeline/registry.rs
git commit -m "test(core): add execute_pipeline unit tests"
```

---

### Task 4: 全量编译和测试验证

- [ ] **Step 1: 编译整个 workspace**

Run: `cargo build`
Expected: BUILD SUCCEEDED

- [ ] **Step 2: 运行全部测试**

Run: `cargo test`
Expected: 所有测试 PASS
