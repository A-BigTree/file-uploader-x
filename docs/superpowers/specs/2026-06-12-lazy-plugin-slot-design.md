# UploadPluginInfo 插件懒加载设计

## 背景

当前 `UploadPluginInfo` 在构造函数（`new_in_process` / `new_from_dylib_path`）中立即创建 `PluginSlot` 实例。对于动态库插件，这意味着在构造阶段就已经执行了动态库加载、符号解析、插件实例创建等重量级操作。即使插件后续可能不会被使用，这些资源也已经被消耗。

## 目标

将 `UploadPluginInfo` 中的 `slot` 字段改为单实例懒加载模式，在首次 `execute()` 或 `on_load()` 调用时才真正初始化插件实例。

## 方案概述

引入新类型 `LazyPluginSlot`，封装 `OnceLock<Arc<PluginSlot>>` 和构建参数，对外暴露与 `PluginSlot` 一致的接口。

## 详细设计

### 1. LazySlotSource 枚举

存储构建插件实例所需的全部参数，自包含不依赖外部字段：

```rust
enum LazySlotSource {
    InProcess {
        config_path: String,
        plugin: Arc<dyn UploadPlugin>,
    },
    Dylib {
        config_path: String,
        dylib_path: String,
    },
}
```

### 2. LazyPluginSlot 结构体

```rust
pub struct LazyPluginSlot {
    source: LazySlotSource,
    inner: OnceLock<Arc<PluginSlot>>,
}
```

### 3. LazyPluginSlot 方法

```rust
impl LazyPluginSlot {
    pub(crate) fn get_or_init(&self) -> Result<&Arc<PluginSlot>, UploadError> {
        self.inner.get_or_try_init(|| {
            let slot: Arc<PluginSlot> = match &self.source {
                LazySlotSource::InProcess { plugin, .. } => {
                    Arc::new(PluginSlot::InProcess(plugin.clone()))
                }
                LazySlotSource::Dylib { dylib_path, .. } => {
                    // 从 dylib_path 加载动态库（与当前 new_from_dylib_path 步骤 4-7 相同）
                }
            };
            slot.on_load();
            Ok(slot)
        })
    }

    pub fn execute(&self, ctx: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        let slot = self.get_or_init()?;
        Ok(slot.execute(ctx))
    }

    pub fn on_load(&self) -> Result<(), UploadError> {
        self.get_or_init()?;
        Ok(())
    }

    pub fn on_unload(&self) {
        if let Some(slot) = self.inner.get() {
            slot.on_unload();
        }
    }
}
```

**关键行为**：
- `get_or_init()` 首次调用时初始化并自动调用 `on_load()`，后续调用直接返回缓存
- `execute()` 和 `on_load()` 返回 `Result`（因为初始化可能失败）
- `on_unload()` 只在已初始化时调用（未初始化则无操作）
- `LazyPluginSlot` 不需要实现 `Drop`，`PluginSlot` 被 drop 时自动调用 `on_unload`（见第 5 节选择 A）

### 4. UploadPluginInfo 改造

```rust
#[derive(Serialize)]
pub struct UploadPluginInfo {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub default_config: Option<Arc<HashMap<String, PluginConfig>>>,
    pub path: String,
    #[serde(skip)]
    pub slot: LazyPluginSlot,  // 从 Arc<PluginSlot> 改为 LazyPluginSlot
}
```

**构造函数变化**：

- `new_in_process`：仍然解析配置文件获取 meta 和 default_config（轻量操作），但不再创建 `PluginSlot`，改为创建 `LazyPluginSlot { source: LazySlotSource::InProcess { config_path, plugin }, inner: OnceLock::new() }`
- `new_from_dylib_path`：仍然解析配置文件获取 meta 和 default_config（轻量操作），但不再加载动态库和解析符号，改为创建 `LazyPluginSlot { source: LazySlotSource::Dylib { config_path, dylib_path }, inner: OnceLock::new() }`

**方法签名变化**：

- `execute(&self, ctx: &UploadInputCtx)` → `Result<UploadOutputCtx, UploadError>`
- `on_load(&self)` → `Result<(), UploadError>`

### 5. PluginSlot Drop 策略

`PluginSlot::Drop` 保持现有 `on_unload` 调用不变。`LazyPluginSlot` 不需要实现 `Drop`——当 `LazyPluginSlot` 被 drop 时，其内部的 `OnceLock<Arc<PluginSlot>>` 被 drop，`Arc` 引用计数归零时 `PluginSlot` drop 自动触发 `on_unload`。

这样避免了 `LazyPluginSlot::Drop` 和 `PluginSlot::Drop` 重复调用 `on_unload` 的问题。

### 6. registry.rs 改造

**PluginRegistryInfo**：

- `execute` 返回类型变为 `Result<UploadOutputCtx, UploadError>`
- `on_load` 返回类型变为 `Result<(), UploadError>`
- `on_unload` 签名不变

**UploadPluginRegistryTable**：

- `new` 中不再循环调用 `on_load()`（已移到懒加载自动触发）
- 新增 `preload_all` 方法：

```rust
pub fn preload_all(&self) -> Result<(), Vec<UploadError>> {
    let errors: Vec<UploadError> = self.plugins
        .iter()
        .filter_map(|p| p.plugin_instance.slot.get_or_init().err())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
```

### 7. 测试策略

- 为 `LazyPluginSlot` 编写单元测试，验证：
  - 首次 `execute` 触发初始化并自动调用 `on_load`
  - 首次 `on_load` 触发初始化
  - 多次调用只初始化一次
  - `on_unload` 只在已初始化时调用
  - Dylib 加载失败的错误处理
- 更新 `registry.rs` 中已有测试，适配新的返回类型
- 更新 `UploadPluginRegistryTable::new` 相关测试（不再自动调用 `on_load`）

## 影响范围

| 文件 | 变更类型 |
|------|----------|
| `file_uploader_core/src/pipeline/plugin.rs` | 新增 `LazySlotSource`、`LazyPluginSlot`；修改 `UploadPluginInfo` 结构体和方法 |
| `file_uploader_core/src/pipeline/registry.rs` | 修改 `PluginRegistryInfo` 方法签名；修改 `UploadPluginRegistryTable::new`；新增 `preload_all` |
| `file_uploader_core/src/main.rs` | 第 35 行 `plugin.slot.execute(&ctx)` → 适配返回 `Result`；第 46 行 `dylib_plugin.slot.execute(&ctx)` → 适配返回 `Result`。两处均需处理 `Result<UploadOutputCtx, UploadError>` 返回值 |
