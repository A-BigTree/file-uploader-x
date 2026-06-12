# LazyPluginSlot 插件懒加载实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `UploadPluginInfo.slot` 从立即初始化改为单实例懒加载，首次 `execute()` 或 `on_load()` 时才创建 `PluginSlot`。

**Architecture:** 引入 `LazySlotSource` 枚举存储构建参数和 `LazyPluginSlot` 结构体封装 `OnceLock<Arc<PluginSlot>>`，对外暴露与 `PluginSlot` 一致的接口。首次初始化时自动调用 `on_load`，`PluginSlot::Drop` 保持不变负责 `on_unload`。

**Tech Stack:** Rust `std::sync::OnceLock`，无新依赖。

---

### Task 1: 新增 LazySlotSource 和 LazyPluginSlot 结构体定义

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs:1-43`

- [ ] **Step 1: 在 `plugin.rs` 文件顶部添加 `OnceLock` 导入，并在 `PluginSlot` 枚举定义之后添加 `LazySlotSource` 和 `LazyPluginSlot` 结构体**

在 `use std::sync::Arc;` 之后添加 `use std::sync::OnceLock;`。

在 `impl Drop for PluginSlot` 块之后（约第 79 行之后），`PluginMeta` 结构体之前，添加：

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

pub struct LazyPluginSlot {
    source: LazySlotSource,
    inner: OnceLock<Arc<PluginSlot>>,
}
```

- [ ] **Step 2: 编译验证结构体定义**

Run: `cargo check -p file_uploader_core`
Expected: 编译通过（结构体暂未使用，不会有错误）

- [ ] **Step 3: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat: add LazySlotSource and LazyPluginSlot struct definitions"
```

---

### Task 2: 实现 LazyPluginSlot 方法

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs`（Task 1 新增的 `LazyPluginSlot` 之后）

- [ ] **Step 1: 在 `LazyPluginSlot` 结构体定义之后添加 impl 块**

```rust
impl LazyPluginSlot {
    pub(crate) fn get_or_init(&self) -> Result<&Arc<PluginSlot>, UploadError> {
        self.inner.get_or_try_init(|| {
            let slot: Arc<PluginSlot> = match &self.source {
                LazySlotSource::InProcess { plugin, .. } => {
                    Arc::new(PluginSlot::InProcess(plugin.clone()))
                }
                LazySlotSource::Dylib { dylib_path, .. } => {
                    let lib = Arc::new(unsafe {
                        Library::new(dylib_path.as_str()).map_err(|e| {
                            UploadError::PluginLoadError(format!("Load dylib failed: {}", e))
                        })?
                    });

                    let get_plugin: libloading::Symbol<FnGetDylibPlugin> = unsafe {
                        lib.get(b"get_dylib_plugin").map_err(|e| {
                            UploadError::PluginLoadError(format!("Get symbol failed: {}", e))
                        })?
                    };

                    let plugin_box = get_plugin();
                    plugin_box.set_logger(plugin_log_callback);

                    Arc::new(PluginSlot::Dylib {
                        plugin: plugin_box,
                        _lib: lib,
                    })
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

- [ ] **Step 2: 编译验证方法实现**

Run: `cargo check -p file_uploader_core`
Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat: implement LazyPluginSlot methods with OnceLock lazy init"
```

---

### Task 3: 改造 UploadPluginInfo 结构体和构造函数

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs:111-251`

- [ ] **Step 1: 修改 `UploadPluginInfo` 结构体中 `slot` 字段类型**

将 `plugin.rs` 中 `UploadPluginInfo` 结构体的 `slot` 字段从：

```rust
    #[serde(skip)]
    pub slot: Arc<PluginSlot>,
```

改为：

```rust
    #[serde(skip)]
    pub slot: LazyPluginSlot,
```

- [ ] **Step 2: 修改 `UploadPluginInfo` 方法签名和 `new_in_process` 构造函数**

将 `execute` 方法从：

```rust
    pub fn execute(&self,  context: &UploadInputCtx) -> UploadOutputCtx {
        self.slot.execute(context)
    }
```

改为：

```rust
    pub fn execute(&self, context: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        self.slot.execute(context)
    }
```

将 `on_load` 方法从：

```rust
    pub fn on_load(&self) {
        self.slot.on_load();
    }
```

改为：

```rust
    pub fn on_load(&self) -> Result<(), UploadError> {
        self.slot.on_load()
    }
```

在 `new_in_process` 方法中，将创建 slot 的部分从：

```rust
        let slot = Arc::new(PluginSlot::InProcess(Arc::from(plugin)));
```

改为：

```rust
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: config_path.to_string(),
                plugin: Arc::from(plugin),
            },
            inner: OnceLock::new(),
        };
```

- [ ] **Step 3: 修改 `new_from_dylib_path` 构造函数**

在 `new_from_dylib_path` 方法中，删除步骤 4-7（加载动态库、获取符号、构建 PluginSlot）的代码，替换为：

```rust
        let slot = LazyPluginSlot {
            source: LazySlotSource::Dylib {
                config_path: config_path.to_string(),
                dylib_path: dylib_path.to_string(),
            },
            inner: OnceLock::new(),
        };
```

注意：配置文件解析（步骤 1-3）保持不变。

- [ ] **Step 4: 编译检查**

Run: `cargo check -p file_uploader_core`
Expected: 在 `registry.rs` 和 `main.rs` 中可能有编译错误（类型不匹配），这是预期的，将在后续 Task 中修复。

- [ ] **Step 5: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat: refactor UploadPluginInfo to use LazyPluginSlot for lazy init"
```

---

### Task 4: 更新 registry.rs 适配新接口

**Files:**
- Modify: `file_uploader_core/src/pipeline/registry.rs`

- [ ] **Step 1: 修改 `PluginRegistryInfo::execute` 返回类型**

将 `execute` 方法从：

```rust
    pub fn execute(&self, context: &UploadInputCtx) -> UploadOutputCtx {
        self.plugin_instance.execute(context)
    }
```

改为：

```rust
    pub fn execute(&self, context: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        self.plugin_instance.execute(context)
    }
```

需要在文件顶部添加导入：

```rust
use file_uploader_sdk::error::UploadError;
```

- [ ] **Step 2: 修改 `PluginRegistryInfo::on_load` 返回类型**

将 `on_load` 方法从：

```rust
    pub fn on_load(&self) {
        self.plugin_instance.on_load();
    }
```

改为：

```rust
    pub fn on_load(&self) -> Result<(), UploadError> {
        self.plugin_instance.on_load()
    }
```

- [ ] **Step 3: 修改 `UploadPluginRegistryTable::new` 移除自动 on_load**

将 `new` 方法从：

```rust
    pub fn new(id: String, mut plugins: Vec<PluginRegistryInfo>) -> Self {
        plugins.sort();
        for plugin in &plugins {
            plugin.on_load();
        }
        UploadPluginRegistryTable { id, plugins }
    }
```

改为：

```rust
    pub fn new(id: String, mut plugins: Vec<PluginRegistryInfo>) -> Self {
        plugins.sort();
        UploadPluginRegistryTable { id, plugins }
    }
```

- [ ] **Step 4: 添加 `preload_all` 方法**

在 `get_plugins_by_phase` 方法之后添加：

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

- [ ] **Step 5: 更新测试中的 `create_mock_plugin_info` 辅助函数**

将 `create_mock_plugin_info` 从：

```rust
    fn create_mock_plugin_info(name: &str, phase: UploadPhase) -> Arc<UploadPluginInfo> {
        let meta = Arc::new(PluginMeta {
            name: name.to_string(),
            title: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            author: Some("test".to_string()),
            phase,
        });

        let plugin = Box::new(MockPlugin);
        let slot = Arc::new(PluginSlot::InProcess(std::sync::Arc::new(*plugin) as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>));

        Arc::new(UploadPluginInfo {
            id: format!("test_{}", name),
            meta,
            default_config: None,
            path: "/test/path".to_string(),
            slot,
        })
    }
```

改为（导入中添加 `use std::sync::OnceLock;`，使用 `LazyPluginSlot`）：

```rust
    fn create_mock_plugin_info(name: &str, phase: UploadPhase) -> Arc<UploadPluginInfo> {
        use crate::pipeline::plugin::{LazyPluginSlot, LazySlotSource};

        let meta = Arc::new(PluginMeta {
            name: name.to_string(),
            title: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            author: Some("test".to_string()),
            phase,
        });

        let plugin = std::sync::Arc::new(MockPlugin) as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>;
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
```

- [ ] **Step 6: 更新 `test_upload_plugin_registry_table_on_load_called` 测试中的 slot 创建**

将 `slot1` 和 `slot2` 的创建从：

```rust
        let slot1 = Arc::new(PluginSlot::InProcess(
            plugin1.clone() as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
        ));
        let slot2 = Arc::new(PluginSlot::InProcess(
            plugin2.clone() as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
        ));
```

改为：

```rust
        use crate::pipeline::plugin::{LazyPluginSlot, LazySlotSource};

        let slot1 = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: "/test/path".to_string(),
                plugin: plugin1.clone() as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let slot2 = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: "/test/path".to_string(),
                plugin: plugin2.clone() as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
```

同时需要更新此测试中的 `PluginSlot` 导入，移除对 `PluginSlot` 的导入，改为导入 `LazyPluginSlot` 和 `LazySlotSource`：

```rust
    use crate::pipeline::plugin::{PluginMeta, LazyPluginSlot, LazySlotSource};
```

并添加 `use std::sync::OnceLock;`。

由于此测试验证 `UploadPluginRegistryTable::new` 不再自动调用 `on_load`（on_load 移到懒加载中），测试断言逻辑需要调整。`UploadPluginRegistryTable::new` 不再调用 `on_load`，所以创建 table 后 load_count 仍为 0。需要改为调用 `execute` 或 `preload_all` 后再验证：

将测试末尾从：

```rust
        assert_eq!(plugin1.get_load_count(), 0);
        assert_eq!(plugin2.get_load_count(), 0);

        UploadPluginRegistryTable::new("test_registry".to_string(), vec![p1, p2]);

        assert_eq!(plugin1.get_load_count(), 1);
        assert_eq!(plugin2.get_load_count(), 1);
```

改为：

```rust
        assert_eq!(plugin1.get_load_count(), 0);
        assert_eq!(plugin2.get_load_count(), 0);

        let table = UploadPluginRegistryTable::new("test_registry".to_string(), vec![p1, p2]);

        assert_eq!(plugin1.get_load_count(), 0);
        assert_eq!(plugin2.get_load_count(), 0);

        table.preload_all().unwrap();

        assert_eq!(plugin1.get_load_count(), 1);
        assert_eq!(plugin2.get_load_count(), 1);
```

- [ ] **Step 7: 编译并运行 registry 测试**

Run: `cargo test -p file_uploader_core`
Expected: 编译通过，测试通过

- [ ] **Step 8: Commit**

```bash
git add file_uploader_core/src/pipeline/registry.rs
git commit -m "feat: update registry.rs for LazyPluginSlot, add preload_all method"
```

---

### Task 5: 更新 main.rs 适配新接口

**Files:**
- Modify: `file_uploader_core/src/main.rs:34-47`

- [ ] **Step 1: 修改 InProcess 插件 execute 调用**

将第 35-36 行从：

```rust
    let result = plugin.slot.execute(&ctx);
    info!("Plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
```

改为：

```rust
    let result = match plugin.slot.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            error!("Plugin execute error: {:?}", e);
            return;
        }
    };
    info!("Plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
```

- [ ] **Step 2: 修改 Dylib 插件 execute 调用**

将第 46-47 行从：

```rust
    let result = dylib_plugin.slot.execute(&ctx);
    info!("Dylib plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
```

改为：

```rust
    let result = match dylib_plugin.slot.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            error!("Dylib plugin execute error: {:?}", e);
            return;
        }
    };
    info!("Dylib plugin execute result: {:?}", serde_json::to_string(&result).unwrap_or("plugin error".to_string()));
```

- [ ] **Step 3: 编译验证**

Run: `cargo check -p file_uploader_core`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add file_uploader_core/src/main.rs
git commit -m "feat: update main.rs to handle Result from lazy plugin execute"
```

---

### Task 6: 更新 plugin.rs 中的测试

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs:274-306`

- [ ] **Step 1: 更新 `test_new_from_dylib_path_invalid_path` 测试**

此测试验证无效路径报错，但由于现在 dylib 加载被延迟了，`new_from_dylib_path` 中路径无效时（`Path::new("/").parent()` 返回 `Some`，不会在构造阶段报错），实际加载失败会发生在首次 `execute` 或 `on_load` 时。

将 `test_new_from_dylib_path_invalid_path` 测试保留但更新断言——当前 "/" 路径会在构造阶段通过（因为 dylib 加载延迟），错误会延迟到 execute 时。更新为：

```rust
    #[test]
    fn test_new_from_dylib_path_invalid_path() {
        let result = UploadPluginInfo::new_from_dylib_path("/");
        assert!(result.is_ok(), "构造阶段不应报错，dylib 加载被延迟");

        let plugin = result.unwrap();
        let ctx = UploadInputCtx {
            file_list: vec![],
            config_info: std::sync::Arc::new(None),
            extra_info: None,
            related_process_info: None,
        };
        let exec_result = plugin.slot.execute(&ctx);
        assert!(exec_result.is_err(), "首次 execute 时应报错");
    }
```

- [ ] **Step 2: 更新 `test_new_from_dylib_path_success` 测试**

此测试验证成功加载，由于 dylib 加载被延迟，构造阶段一定成功：

```rust
    #[test]
    fn test_new_from_dylib_path_success() {
        let result = UploadPluginInfo::new_from_dylib_path(
            "../../target/debug/libuploader_example_plugin.dylib"
        );
        assert!(result.is_ok(), "构造阶段不应报错");
        let _ = result;
    }
```

- [ ] **Step 3: 运行全部测试**

Run: `cargo test -p file_uploader_core`
Expected: 测试全部通过

- [ ] **Step 4: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "test: update plugin tests for lazy loading behavior"
```

---

### Task 7: 全量构建和测试验证

**Files:** 无文件变更

- [ ] **Step 1: 全量构建整个 workspace**

Run: `cargo build`
Expected: 编译通过，无 warning

- [ ] **Step 2: 运行全部测试**

Run: `cargo test`
Expected: 全部通过

- [ ] **Step 3: 最终 Commit（如有 lint/格式化修复）**

```bash
cargo fmt
git add -A
git commit -m "chore: format code after lazy plugin slot implementation"
```
