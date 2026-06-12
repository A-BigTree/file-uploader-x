# Pipeline 执行引擎 + 回调机制设计

## 概述

为 `UploadPluginRegistryTable` 新增 `execute_pipeline` 方法，按阶段顺序执行已注册插件，支持阶段级和插件级回调通知，并在插件执行失败时中断流水线。

## 新增类型

### PipelineEventKind

定义在 `file_uploader_sdk/src/models/interface.rs`：

```rust
pub enum PipelineEventKind {
    PhaseStart,
    PhaseEnd,
    PluginStart,
    PluginEnd,
}
```

### PipelineEvent

```rust
pub struct PipelineEvent<'a> {
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
    pub plugin_meta: Option<&'a PluginMeta>,
}
```

- `PhaseStart` / `PhaseEnd` 时：`plugin_id` 和 `plugin_meta` 为 `None`
- `PluginStart` / `PluginEnd` 时：`plugin_id` 和 `plugin_meta` 为 `Some`

### PipelineCallback trait

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

- `PhaseStart` / `PluginStart`：`result` 为 `None`
- `PhaseEnd` / `PluginEnd`：`result` 为 `Some`，包含该阶段或插件的输出

## execute_pipeline 方法

### 签名

```rust
impl UploadPluginRegistryTable {
    pub fn execute_pipeline(
        &self,
        input_ctx: UploadInputCtx,
        callback: Option<&dyn PipelineCallback>,
    ) -> UploadOutputCtx
}
```

### 执行逻辑

```
输入: UploadInputCtx
  │
  ├─ 遍历阶段 [Input, PreUpload, Upload, PostUpload, Output]
  │   │
  │   ├─ get_plugins_by_phase(phase) → 该阶段已启用插件列表（按 priority 排序）
  │   │   │
  │   │   └─ 无插件 → 跳过该阶段
  │   │
  │   ├─ 回调 on_event(PhaseStart) — ctx=当前InputCtx, result=None
  │   │
  │   ├─ 链式执行阶段内插件：
  │   │   ├─ 构建插件 InputCtx：
  │   │   │   - file_list: 从上一插件输出继承（首个插件用传入的 input_ctx）
  │   │   │   - config_info: Arc::new(plugin.registry_config.clone())
  │   │   │   - extra_info: 每次插件执行完后，将 output.extra_info 合并更新（output key 覆盖已有 key）
  │   │   │   - related_process_info: 从传入的 input_ctx 继承
  │   │   │
  │   │   ├─ 回调 on_event(PluginStart) — ctx=插件InputCtx, result=None
  │   │   ├─ 执行 plugin.execute(&ctx)
  │   │   ├─ 回调 on_event(PluginEnd) — ctx=插件InputCtx, result=Some(输出)
  │   │   │
  │   │   └─ 检查结果：
  │   │       ├─ Err(UploadError) → 中断，返回失败的 OutputCtx
  │   │       ├─ Ok(OutputCtx { result: Failed, .. }) → 中断，返回该 OutputCtx
  │   │       └─ Ok(OutputCtx { result: Success/Interrupt, .. }) → 继续下一插件
  │   │
  │   └─ 回调 on_event(PhaseEnd) — ctx=最后一个插件InputCtx, result=Some(最后输出)
  │       （如果阶段被中断，不触发 PhaseEnd）
  │
  └─ 返回最终 UploadOutputCtx
      （如果所有阶段均无插件，返回默认成功的 OutputCtx）
```

### 中断规则

1. 插件 `execute` 返回 `Err(UploadError)`：包装为失败的 `UploadOutputCtx` 并中断
2. 插件返回 `OutputCtx { result: Failed }`：直接中断
3. 中断时：当前插件的 `PluginEnd` 回调已触发，但不触发该阶段的 `PhaseEnd`，也不执行后续阶段

### OutputCtx → InputCtx 转换（阶段间/插件间链式传递）

```rust
fn output_to_input(
    output: &UploadOutputCtx,
    registry_config: Option<Value>,
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
        config_info: Arc::new(registry_config),
        extra_info: if extra_info.is_empty() { None } else { Some(extra_info) },
        related_process_info: source_ctx.related_process_info.clone(),
    }
}
```

- `file_list`：从上一输出继承，若无则用空 Vec
- `config_info`：每个插件使用其 `registry_config`（注册时配置）
- `extra_info`：每次插件执行完后，将 output 的 `extra_info` 合并到当前 `extra_info` 中（output 的 key 覆盖已有 key），传递给下一个插件
- `related_process_info`：从传入的 input_ctx 继承

## 文件变更清单

| 文件 | 变更内容 |
|------|----------|
| `file_uploader_sdk/src/models/interface.rs` | 新增 `PipelineEventKind`、`PipelineEvent<'a>`、`PipelineCallback` trait |
| `file_uploader_sdk/src/models.rs` | 导出 `PipelineEventKind`、`PipelineEvent`、`PipelineCallback` |
| `file_uploader_core/src/pipeline/registry.rs` | 新增 `execute_pipeline` 方法 + `output_to_input` 辅助函数 |

## 测试计划

1. **空注册表执行**：无插件时应返回默认成功 OutputCtx
2. **单阶段单插件**：验证回调触发顺序（PhaseStart → PluginStart → PluginEnd → PhaseEnd）
3. **多阶段多插件链式传递**：验证 file_list 在插件间正确传递
4. **插件执行失败中断**：验证 Err 导致中断，后续插件不执行，PhaseEnd 不触发
5. **插件返回 Failed 中断**：验证 OutputResultType::Failed 同样中断
6. **registry_config 注入**：验证每个插件的 InputCtx.config_info 来自 registry_config
7. **无回调执行**：callback 为 None 时正常执行，无 panic
8. **跳过无插件阶段**：某阶段无插件时不触发 PhaseStart/PhaseEnd
