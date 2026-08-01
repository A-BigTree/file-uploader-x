# 进程内插件目录入口（catalog）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `file_uploader_core` 提供「列出所有进程内插件信息」+「按 ID 取插件实现对象」两个入口能力，供上层应用一行调用；以显式清单作为进程内插件的发现机制。

**Architecture:** 方案 C——`file_uploader_plugins` 维护显式清单 `list_in_process_plugins() -> &'static [InProcessEntry]`（单一注册点）；`core::pipeline::in_process_catalog` 提供 `InProcessPluginCatalog` struct（`load_default` / `load_from` / `list` / `get`）+ 全局便捷函数。`list` 返回轻量概要 `PluginInfoSummary`（不可 execute），`get(id)` 返回 `Arc<dyn UploadPlugin>`（per-id `OnceLock` 单例复用，`on_load` 由调用方显式管理）。ID 由 `meta.json` 派生 `in_process_{phase:?}_{name}`。资源根路径经 `build.rs` 注入 env! + `resources_root()` 访问器解决跨 crate 传递。

**Tech Stack:** Rust 2024 / serde / thiserror / tracing。无新外部依赖。

**规格依据：** `docs/superpowers/specs/2026-08-01-in-process-plugin-catalog-design.md`

**关键约定：**
- `InProcessEntry.factory` 类型为 `fn() -> Arc<dyn UploadPlugin>`（函数指针，**非捕获**）；清单里写法为 `|| -> Arc<dyn UploadPlugin> { Arc::new(X) }`（闭包带显式返回类型以触发 unsized coercion，再自动 coerce 为 fn 指针）。
- ID 派生必须与 `UploadPluginInfo::new_in_process` 完全一致：`format!("in_process_{:?}_{}", meta.phase, meta.name)`。
- `get` **不自动**调 `on_load`；`list` / `get` 不触发插件加载（除 `get` 首次会执行 factory 产出实例，但不调 `on_load`）。
- 资源加载失败 = 硬失败（返回 `Err(UploadError::PluginLoadError(..))`）。
- 每个任务末尾提交；提交信息用 conventional commits（英文）。

**验证命令速查：**
- 单 crate：`cargo test -p file_uploader_plugins`、`cargo test -p file_uploader_core`
- 编译：`cargo build -p file_uploader_plugins`、`cargo build -p file_uploader_core`
- 全量：`cargo build` && `cargo test`
- Lint：`cargo clippy --workspace -- -D warnings`（若项目已配置）

---

## Task 1: build.rs 注入资源根 env + `resources_root()` 访问器

**Files:**
- Modify: `file_uploader_plugins/build.rs`
- Modify: `file_uploader_plugins/src/lib.rs`

- [ ] **Step 1: 写失败测试（追加到 `src/lib.rs` 末尾）**

在 `file_uploader_plugins/src/lib.rs` 末尾追加：

```rust
/// 内置进程内插件的资源根目录（编译期由 build.rs 注入）。
/// 指向 `target/<profile>/resources`。
#[cfg(test)]
mod tests {
    use super::resources_root;

    #[test]
    fn resources_root_is_nonempty_and_ends_with_resources() {
        let r = resources_root();
        assert!(!r.is_empty(), "resources_root must be injected by build.rs");
        assert!(
            r.ends_with("resources"),
            "resources_root should end with 'resources', got: {r}"
        );
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_plugins tests::resources_root`
Expected: 编译失败，`cannot find function 'resources_root'`（尚未定义）

- [ ] **Step 3: build.rs 注入 env**

在 `file_uploader_plugins/build.rs` 的 `let dst = target_dir.join("resources");` 之后插入一行（紧接 `if src.exists()` 之前）：

```rust
    println!("cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR={}", dst.display());
```

> 完整 `main()` 上下文（确认插入位置）：`dst` 计算在 `eprintln!("target_dir = ...")` 之后。

- [ ] **Step 4: lib.rs 加 `resources_root()`**

在 `file_uploader_plugins/src/lib.rs` 顶部 `pub mod` 声明之后追加：

```rust
use std::sync::Arc;
use file_uploader_sdk::models::interface::UploadPlugin;

/// 内置进程内插件的资源根目录（编译期由 build.rs 注入），指向 `target/<profile>/resources`。
/// 供 `file_uploader_core` 定位进程内插件资源，避免宿主硬编码 `target/debug`。
pub fn resources_root() -> &'static str {
    env!("FILE_UPLOADER_RESOURCES_DIR")
}
```

> `Arc` / `UploadPlugin` 的 import 供 Task 2 的 `InProcessEntry` 复用，此处先引入。

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p file_uploader_plugins tests::resources_root`
Expected: 1 passed

- [ ] **Step 6: 提交**

```bash
git add file_uploader_plugins/build.rs file_uploader_plugins/src/lib.rs
git commit -m "feat(plugins): inject FILE_UPLOADER_RESOURCES_DIR env and expose resources_root()"
```

---

## Task 2: `InProcessEntry` + `list_in_process_plugins()` 清单

**Files:**
- Modify: `file_uploader_plugins/src/lib.rs`

- [ ] **Step 1: 写失败测试（追加到 `src/lib.rs` 的 `mod tests`）**

在 Task 1 新建的 `mod tests` 内追加：

```rust
    use super::{list_in_process_plugins, InProcessEntry};

    #[test]
    fn manifest_includes_known_builtin_plugins() {
        let entries = list_in_process_plugins();
        assert!(entries.len() >= 2, "should list at least 2 builtin plugins");

        let subdirs: Vec<&str> = entries.iter().map(|e| e.resource_subdir).collect();
        assert!(
            subdirs.contains(&"input/default_input_handler"),
            "missing default_input_handler, got: {subdirs:?}"
        );
        assert!(
            subdirs.contains(&"pre/upload_file_validator"),
            "missing upload_file_validator, got: {subdirs:?}"
        );
    }

    #[test]
    fn manifest_is_nonempty_static_slice() {
        let e1 = list_in_process_plugins();
        let e2 = list_in_process_plugins();
        assert!(!e1.is_empty());
        assert!(std::ptr::eq(e1.as_ptr(), e2.as_ptr()), "should be the same 'static slice");
    }

    #[test]
    fn entry_factory_yields_plugin_with_correct_name() {
        // factory 必须可重复调用且产出实现 UploadPlugin 的对象
        let entries = list_in_process_plugins();
        let input_entry = entries
            .iter()
            .find(|e| e.resource_subdir == "input/default_input_handler")
            .expect("default_input_handler entry present");
        let p = (input_entry.factory)();
        assert_eq!(p.name(), "default_input_handler");
        assert!(matches!(p.phase(), file_uploader_sdk::models::enums::UploadPhase::Input));
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p file_uploader_plugins tests::manifest`
Expected: 编译失败，`cannot find type 'InProcessEntry'` / `cannot find function 'list_in_process_plugins'`

- [ ] **Step 3: 实现 `InProcessEntry` 与清单**

在 `file_uploader_plugins/src/lib.rs`（`resources_root()` 之后）追加：

```rust
/// 单个进程内插件的注册条目（编译期固定）。
///
/// 新增插件只需在 [`list_in_process_plugins`] 末尾追加一条 —— 这是进程内插件的
/// **唯一注册点**。
pub struct InProcessEntry {
    /// 资源目录相对 resources 根的子路径，如 `"input/default_input_handler"`。
    /// 必须与 `resources/<phase>/<name>` 实际目录一致。
    pub resource_subdir: &'static str,
    /// 插件实例工厂（非捕获，可重复调用；由 catalog 内部 `OnceLock` 单例化）。
    pub factory: fn() -> Arc<dyn UploadPlugin>,
}

/// 所有内置进程内插件的显式清单。
///
/// 新增插件在此追加一条 `InProcessEntry` 即可被 `InProcessPluginCatalog` 发现。
pub fn list_in_process_plugins() -> &'static [InProcessEntry] {
    &[
        InProcessEntry {
            resource_subdir: "input/default_input_handler",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(input::default_input_handler::DefaultInputHandler)
            },
        },
        InProcessEntry {
            resource_subdir: "pre/upload_file_validator",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(pre_upload::upload_file_validator::UploadFileValidator)
            },
        },
        // 后续 common_uploader / common_output 落地后在此追加
    ]
}
```

> 注意 factory 的显式返回类型标注 `-> Arc<dyn UploadPlugin>` 不可省略——否则 `Arc::new(X)` 推断为 `Arc<X>`，无法 coerce 到 fn 指针的目标返回类型。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p file_uploader_plugins tests::manifest`
Expected: 3 passed

- [ ] **Step 5: 全量编译确认无破坏**

Run: `cargo build -p file_uploader_plugins`
Expected: 编译通过

- [ ] **Step 6: 提交**

```bash
git add file_uploader_plugins/src/lib.rs
git commit -m "feat(plugins): add InProcessEntry manifest and list_in_process_plugins()"
```

---

## Task 3: `in_process_catalog` 核心模块（DTO + Catalog + 测试）

**Files:**
- Create: `file_uploader_core/src/pipeline/in_process_catalog.rs`
- Modify: `file_uploader_core/src/pipeline.rs`

- [ ] **Step 1: 注册新模块（先让空文件可编译）**

在 `file_uploader_core/src/pipeline.rs` 末尾追加一行：

```rust
pub mod in_process_catalog;
```

创建空文件 `file_uploader_core/src/pipeline/in_process_catalog.rs`（占位，下一步填充）。

Run: `cargo build -p file_uploader_core`
Expected: 编译通过（空模块）

- [ ] **Step 2: 写失败测试（写入 `in_process_catalog.rs`）**

把 `file_uploader_core/src/pipeline/in_process_catalog.rs` 全文替换为：

```rust
use crate::pipeline::plugin::{PluginConfigInfo, PluginMeta, PluginResource};
use file_uploader_plugins::InProcessEntry;
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
use file_uploader_sdk::models::interface::UploadPlugin;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

/// 进程内插件的展示信息（不可 execute，纯展示）。
#[derive(Serialize, Clone)]
pub struct PluginInfoSummary {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    pub readme_path: Option<String>,
}

pub struct InProcessPluginCatalog {
    summaries: Vec<PluginInfoSummary>,
    instances: HashMap<String, OnceLock<Arc<dyn UploadPlugin>>>,
    factories: HashMap<String, fn() -> Arc<dyn UploadPlugin>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 测试夹具 ----

    /// 全局 on_load 计数器（factory 必须是非捕获 fn 指针，故用 static 共享状态）
    static MOCK_LOAD_COUNT: AtomicU32 = AtomicU32::new(0);

    struct CountingMock;
    impl UploadPlugin for CountingMock {
        fn name(&self) -> &'static str { "mock_input" }
        fn phase(&self) -> UploadPhase { UploadPhase::Input }
        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx { result: OutputResultType::Success, message: "ok".into(), file: None, extra_info: None }
        }
        fn on_load(&self) {
            MOCK_LOAD_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 在临时目录下造一个只含 meta.json 的资源子目录，返回资源根。
    fn make_tmp_resource_root(subdir: &str, name: &str, phase: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fux_catalog_{}_{}",
            std::process::id(),
            subdir.replace('/', "_")
        ));
        let dir = root.join(subdir);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = format!(
            r#"{{"name":"{name}","title":"M","description":"d","version":"0.0.1","author":null,"phase":"{phase}"}}"#
        );
        std::fs::write(dir.join("meta.json"), meta).unwrap();
        root
    }

    fn one_mock_catalog() -> (InProcessPluginCatalog, std::path::PathBuf) {
        let root = make_tmp_resource_root("input/mock_input", "mock_input", "Input");
        let cat = InProcessPluginCatalog::from_entries(
            &[InProcessEntry {
                resource_subdir: "input/mock_input",
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            }],
            &root,
        )
        .expect("from_entries with valid tmp resource should succeed");
        (cat, root)
    }

    // ---- 测试用例 ----

    #[test]
    fn list_count_and_derived_id() {
        let (cat, _root) = one_mock_catalog();
        assert_eq!(cat.list().len(), 1);
        assert_eq!(cat.list()[0].id, "in_process_Input_mock_input");
    }

    #[test]
    fn get_returns_singleton_same_arc() {
        let (cat, _root) = one_mock_catalog();
        let a = cat.get("in_process_Input_mock_input").expect("known id");
        let b = cat.get("in_process_Input_mock_input").expect("known id");
        assert!(Arc::ptr_eq(&a, &b), "get must return the same singleton Arc");
    }

    #[test]
    fn get_unknown_id_returns_none() {
        let (cat, _root) = one_mock_catalog();
        assert!(cat.get("does_not_exist").is_none());
    }

    #[test]
    fn get_does_not_trigger_on_load() {
        MOCK_LOAD_COUNT.store(0, Ordering::SeqCst);
        let (cat, _root) = one_mock_catalog();
        let _ = cat.get("in_process_Input_mock_input");
        let _ = cat.get("in_process_Input_mock_input");
        assert_eq!(
            MOCK_LOAD_COUNT.load(Ordering::SeqCst),
            0,
            "get must NOT auto-call on_load"
        );
    }

    #[test]
    fn list_does_not_trigger_on_load() {
        MOCK_LOAD_COUNT.store(0, Ordering::SeqCst);
        let (cat, _root) = one_mock_catalog();
        let _ = cat.list();
        assert_eq!(MOCK_LOAD_COUNT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn from_entries_fails_when_meta_missing() {
        // 空临时根 → 子目录不存在 → PluginResource::load 失败 → 硬失败
        let empty_root = std::env::temp_dir().join(format!("fux_catalog_empty_{}", std::process::id()));
        std::fs::create_dir_all(&empty_root).unwrap();
        let res = InProcessPluginCatalog::from_entries(
            &[InProcessEntry {
                resource_subdir: "input/none",
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            }],
            &empty_root,
        );
        assert!(res.is_err(), "missing meta.json should be a hard failure");
        let _ = std::fs::remove_dir_all(&empty_root);
    }
}
```

- [ ] **Step 3: 运行测试验证失败**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: 编译失败，`no function 'from_entries'`、`no method 'list' / 'get'` 等（struct 字段与方法尚未实现）

- [ ] **Step 4: 实现 Catalog（在 `mod tests` 之前插入实现块）**

在 `in_process_catalog.rs` 的 `pub struct InProcessPluginCatalog { ... }` 定义之后、`#[cfg(test)]` 之前插入：

```rust
impl InProcessPluginCatalog {
    /// 由调用方提供清单与资源根目录构造（测试 / 定制场景入口）。
    pub(crate) fn from_entries(
        entries: &[InProcessEntry],
        resources_root: &Path,
    ) -> Result<Self, UploadError> {
        let mut summaries = Vec::with_capacity(entries.len());
        let mut instances = HashMap::new();
        let mut factories = HashMap::new();

        for entry in entries {
            let dir = resources_root.join(entry.resource_subdir);
            let resource = PluginResource::load(&dir)?; // 硬失败
            // ID 派生：与 UploadPluginInfo::new_in_process 完全一致
            let id = format!("in_process_{:?}_{}", resource.meta.phase, resource.meta.name);

            summaries.push(PluginInfoSummary {
                id: id.clone(),
                meta: resource.meta.clone(),
                config: resource.config.clone(),
                readme_path: resource.readme_path.clone(),
            });
            factories.insert(id.clone(), entry.factory);
            instances.insert(id, OnceLock::new());
        }

        Ok(InProcessPluginCatalog { summaries, instances, factories })
    }

    /// 自定义资源根目录加载（用编译期清单 [`file_uploader_plugins::list_in_process_plugins`]）。
    pub fn load_from(resources_root: &Path) -> Result<Self, UploadError> {
        Self::from_entries(file_uploader_plugins::list_in_process_plugins(), resources_root)
    }

    /// 用编译期 env! 默认资源根目录加载。
    pub fn load_default() -> Result<Self, UploadError> {
        Self::load_from(Path::new(file_uploader_plugins::resources_root()))
    }

    /// 所有进程内插件概要（不触发插件加载、不调 on_load）。
    pub fn list(&self) -> &[PluginInfoSummary] {
        &self.summaries
    }

    /// 按 id 取插件实现对象（per-id 单例复用；on_load 由调用方显式管理）。
    pub fn get(&self, id: &str) -> Option<Arc<dyn UploadPlugin>> {
        let cell = self.instances.get(id)?;
        let factory = *self.factories.get(id)?;
        let arc = cell.get_or_init(factory);
        Some(arc.clone())
    }
}
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: 6 passed

- [ ] **Step 6: 提交**

```bash
git add file_uploader_core/src/pipeline/in_process_catalog.rs file_uploader_core/src/pipeline.rs
git commit -m "feat(core): add InProcessPluginCatalog with list/get and derived id"
```

---

## Task 4: 全局便捷函数 + `lib.rs` 重导出

**Files:**
- Modify: `file_uploader_core/src/pipeline/in_process_catalog.rs`
- Modify: `file_uploader_core/src/lib.rs`

- [ ] **Step 1: 实现全局便捷函数**

在 `in_process_catalog.rs` 的 `impl InProcessPluginCatalog { ... }` 块之后、`#[cfg(test)]` 之前追加：

```rust
/// 全局 catalog 单例（首次成功 load_default 后固定；失败则下次重试）。
static GLOBAL_CATALOG: OnceLock<InProcessPluginCatalog> = OnceLock::new();

fn ensure_catalog() -> Result<&'static InProcessPluginCatalog, UploadError> {
    if let Some(c) = GLOBAL_CATALOG.get() {
        return Ok(c);
    }
    let c = InProcessPluginCatalog::load_default()?; // 失败则不 set，下次调用重试
    let _ = GLOBAL_CATALOG.set(c); // 竞态由 OnceLock 收敛
    Ok(GLOBAL_CATALOG
        .get()
        .expect("GLOBAL_CATALOG must be set after successful load"))
}

/// 列出所有内置进程内插件概要（首次调用触发 load_default）。
pub fn list_in_process_plugins() -> Result<&'static [PluginInfoSummary], UploadError> {
    Ok(ensure_catalog()?.list())
}

/// 按 id 取进程内插件实现对象（首次调用触发 load_default）。
pub fn get_in_process_plugin(
    id: &str,
) -> Result<Option<Arc<dyn UploadPlugin>>, UploadError> {
    Ok(ensure_catalog()?.get(id))
}
```

- [ ] **Step 2: lib.rs 重导出**

把 `file_uploader_core/src/lib.rs` 全文替换为：

```rust
pub mod config;
pub mod pipeline;

pub use pipeline::in_process_catalog::{
    get_in_process_plugin, list_in_process_plugins, InProcessPluginCatalog, PluginInfoSummary,
};
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p file_uploader_core`
Expected: 编译通过，四个符号从 `file_uploader_core` 顶层可访问

- [ ] **Step 4: 单元测试仍通过**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: 6 passed（全局函数留待 Task 5 集成测试覆盖）

- [ ] **Step 5: 提交**

```bash
git add file_uploader_core/src/pipeline/in_process_catalog.rs file_uploader_core/src/lib.rs
git commit -m "feat(core): add global list/get convenience fns and re-export from crate root"
```

---

## Task 5: 集成测试（`load_default` + 全局函数，依赖真实资源）

**Files:**
- Modify: `file_uploader_core/src/pipeline/in_process_catalog.rs`（在 `mod tests` 内追加）

> 该测试依赖 `target/debug/resources` 已由 build.rs 复制就绪；沿用项目既有「skip if !exists」模式（参见 `pipeline/plugin.rs` 的 `test_plugin_resource_load_success`）。

- [ ] **Step 1: 写集成测试（追加到 `mod tests` 末尾）**

在 `in_process_catalog.rs` 的 `mod tests` 内追加：

```rust
    use file_uploader_plugins::resources_root;
    use std::path::PathBuf;

    fn target_resources_root() -> PathBuf {
        PathBuf::from(resources_root())
    }

    #[test]
    fn load_default_lists_real_builtins() {
        let root = target_resources_root();
        if !root.join("input/default_input_handler").exists() {
            eprintln!("skip: {} not ready yet", root.display());
            return;
        }
        let cat = InProcessPluginCatalog::load_default().expect("load_default should succeed");
        let ids: Vec<&str> = cat.list().iter().map(|s| s.id.as_str()).collect();
        assert!(
            ids.iter().any(|id| id.ends_with("default_input_handler")),
            "should include default_input_handler, got: {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id.ends_with("upload_file_validator")),
            "should include upload_file_validator, got: {ids:?}"
        );
    }

    #[test]
    fn load_default_get_returns_real_executable_plugin() {
        let root = target_resources_root();
        if !root.join("input/default_input_handler").exists() {
            eprintln!("skip: {} not ready yet", root.display());
            return;
        }
        let cat = InProcessPluginCatalog::load_default().expect("load_default");
        let id = "in_process_Input_default_input_handler";
        let plugin = cat.get(id).expect("default_input_handler should be present");
        assert_eq!(plugin.name(), "default_input_handler");
        // execute 不 panic（空 ctx）
        let ctx = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: None,
        };
        let out = plugin.execute(&ctx);
        assert!(matches!(out.result, OutputResultType::Success));
    }

    #[test]
    fn global_functions_work_against_real_resources() {
        let root = target_resources_root();
        if !root.join("input/default_input_handler").exists() {
            eprintln!("skip: {} not ready yet", root.display());
            return;
        }
        let listed = super::list_in_process_plugins().expect("global list should succeed");
        assert!(!listed.is_empty());
        let got = super::get_in_process_plugin("in_process_Input_default_input_handler")
            .expect("global get should succeed")
            .expect("plugin should exist");
        assert_eq!(got.name(), "default_input_handler");
    }
```

- [ ] **Step 2: 运行集成测试**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: 9 passed（6 单元 + 3 集成；若 target resources 未就绪则集成 3 条 skip，但仍算 passed）

- [ ] **Step 3: 全量构建与测试**

Run: `cargo build && cargo test`
Expected: 全绿

- [ ] **Step 4: 提交**

```bash
git add file_uploader_core/src/pipeline/in_process_catalog.rs
git commit -m "test(core): add integration tests for load_default and global fns"
```

---

## Task 6: 规范文档同步 —— `plugin-specification.md`

**Files:**
- Modify: `docs/references/plugin-specification.md`

> 纯文档变更，无代码测试。每步用 `grep` 自检定位准确。

- [ ] **Step 1: 新增 §1.7「进程内插件目录入口（catalog）」**

定位插入点：§1.6 末尾（`> **安全**：不要整体序列化 ctx 打日志 ……` 那一段）之后、§2 之前的 `---` 分隔线之前。

在 `docs/references/plugin-specification.md` 中找到：

```
> **安全**：不要整体序列化 `ctx` 打日志 —— 会泄漏 `secret: true` 字段的值。只打印必要的非敏感字段。

---

## 2. 资源目录与 meta.json
```

将其中的 `---`（§1.6 与 §2 之间的分隔）替换为如下内容（即在 `---` 前插入 §1.7 整节）：

```markdown
> **安全**：不要整体序列化 `ctx` 打日志 —— 会泄漏 `secret: true` 字段的值。只打印必要的非敏感字段。

### 1.7 进程内插件目录入口（catalog）

进程内插件经**显式清单**登记（新增插件的唯一注册点）：

```rust
// file_uploader_plugins
pub struct InProcessEntry {
    pub resource_subdir: &'static str,           // 如 "input/default_input_handler"
    pub factory: fn() -> Arc<dyn UploadPlugin>,
}
pub fn list_in_process_plugins() -> &'static [InProcessEntry];
```

`file_uploader_core::pipeline::in_process_catalog` 提供两个面向上层的入口能力：

| 能力 | API | 返回 |
|---|---|---|
| 列出所有进程内插件信息 | `InProcessPluginCatalog::list()` / 全局 `list_in_process_plugins()` | `&[PluginInfoSummary]`（id + meta + config + readme_path，**不可 execute**） |
| 按 ID 取插件实现对象 | `InProcessPluginCatalog::get(id)` / 全局 `get_in_process_plugin(id)` | `Option<Arc<dyn UploadPlugin>>` |

- `InProcessPluginCatalog::load_default()` 用编译期资源根加载；`load_from(path)` 可自定义根（测试/定制）。
- `get` 返回的对象 **per-id 单例复用**（`OnceLock` 缓存），但 **`on_load` / `on_unload` 由调用方显式管理**，catalog 不自动调用。
- 查找 key（id）沿用 [§2 插件 ID 生成](#插件-id-生成) 的进程内规则。

```rust
// 上层一行调用
for p in file_uploader_core::list_in_process_plugins()? {
    println!("{}: {} ({:?})", p.id, p.meta.title, p.meta.phase);
}
let plugin = file_uploader_core::get_in_process_plugin("in_process_Input_default_input_handler")?;
```

---

## 2. 资源目录与 meta.json
```

- [ ] **Step 2: §2「插件 ID 生成」节加交叉引用**

找到：

```
注意进程内 ID 用 `{:?}` 格式化 phase，得到的是 `PreUpload` 这类大驼峰形态。
```

在其后追加一行：

```markdown

> 进程内插件目录入口（§1.7）的 `list` / `get` 沿用此 ID 规则作为查找 key。
```

- [ ] **Step 3: §8.1 补 env! 注入说明**

找到 §8.1 末尾：

```
> **易踩**：改的是源 `resources/`，运行时读的是 `target/.../resources/` 副本 —— 改完要重新 build。
```

在其后追加：

```markdown

`build.rs` 另注入 `cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR=<target>/<profile>/resources`，
由 `file_uploader_plugins::resources_root() -> &'static str` 暴露，
供 `InProcessPluginCatalog::load_default()` 定位资源根（避免宿主硬编码 `target/debug`）。
```

- [ ] **Step 4: 自检三处更新到位**

Run:
```bash
grep -n "1.7 进程内插件目录入口" docs/references/plugin-specification.md
grep -n "沿用此 ID 规则作为查找 key" docs/references/plugin-specification.md
grep -n "FILE_UPLOADER_RESOURCES_DIR" docs/references/plugin-specification.md
```
Expected: 三条命令各命中 ≥ 1 行

- [ ] **Step 5: 提交**

```bash
git add docs/references/plugin-specification.md
git commit -m "docs(spec): add §1.7 in-process catalog, id cross-ref, env! note"
```

---

## Task 7: skill 文档同步 —— `designing-in-process-plugins/SKILL.md`

**Files:**
- Modify: `.agents/skills/designing-in-process-plugins/SKILL.md`

- [ ] **Step 1: 必备文件清单表补一行**

找到「必备文件清单」表格最后一行：

```
| `file_uploader_plugins/build.rs` | 已存在 | 递归复制 `resources/` 整树（新增插件自动覆盖） |
```

在其后追加一行：

```markdown
| `file_uploader_plugins/src/lib.rs` | 新增插件必须 | 在 `list_in_process_plugins()` 登记 `InProcessEntry`（否则不被 catalog 发现） |
```

- [ ] **Step 2: 新增「登记到进程内插件清单」步骤**

找到「### 8. 模块注册」整节内容：

```
### 8. 模块注册

`file_uploader_plugins/src/<phase_dir>.rs` 加 `pub mod <name>;`。
`<phase_dir>` ∈ `input` / `pre_upload` / `upload` / `post_upload`。
```

在其之后插入新步骤（原「### 9. README.md」及之后编号顺延为 10/11/12/13）：

```markdown
### 9. 登记到进程内插件清单

`file_uploader_plugins/src/lib.rs` 的 `list_in_process_plugins()` 末尾追加一条 `InProcessEntry`：

```rust
InProcessEntry {
    resource_subdir: "pre/size_limiter",   // <phase 短名>/<name>，与资源目录一致
    factory: || -> Arc<dyn UploadPlugin> { Arc::new(SizeLimiter) },
},
```

- `resource_subdir` 必须与 `resources/<phase>/<name>` 实际目录一致
- `factory` 闭包必须带显式返回类型 `-> Arc<dyn UploadPlugin>`（否则无法 coerce 为 fn 指针）
- **不登记则不会被 `InProcessPluginCatalog` 发现**，上层「列出 / 按 ID 取」拿不到该插件
```

随后把后续章节标题编号 +1：`### 9. README.md` → `### 10.`、`### 10. 测试（TDD）` → `### 11.`、`### 11. 注册到 pipeline` → `### 12.`。

- [ ] **Step 3: 快速检查清单「代码」段补一项**

找到「## 快速检查清单」下「**代码**」段最后一项：

```
- [ ] `src/<phase_dir>.rs` 注册 `pub mod <name>;`
```

在其后追加：

```markdown
- [ ] 已在 `list_in_process_plugins()` 追加 `InProcessEntry`（`resource_subdir` 与资源目录路径一致）
```

- [ ] **Step 4: 自检三处更新到位**

Run:
```bash
grep -n "list_in_process_plugins" .agents/skills/designing-in-process-plugins/SKILL.md
grep -n "登记到进程内插件清单" .agents/skills/designing-in-process-plugins/SKILL.md
```
Expected: 各命中 ≥ 2 行 / ≥ 1 行

- [ ] **Step 5: 提交**

```bash
git add .agents/skills/designing-in-process-plugins/SKILL.md
git commit -m "docs(skill): require InProcessEntry registration for in-process plugins"
```

---

## 完成确认

- [ ] **Step 1: 全量构建 + 测试**

Run: `cargo build && cargo test`
Expected: 全绿

- [ ] **Step 2: 冒烟（可选）**

Run: `cargo run -p file_uploader_core`
Expected: 正常输出（main.rs 未改动，仍可跑）

- [ ] **Step 3: 文档交叉引用无断链**

Run:
```bash
grep -n "1.7 进程内插件目录入口" docs/references/plugin-specification.md
grep -n "登记到进程内插件清单" .agents/skills/designing-in-process-plugins/SKILL.md
```
Expected: 均命中
