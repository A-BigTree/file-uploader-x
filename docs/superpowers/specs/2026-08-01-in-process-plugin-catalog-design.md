# 进程内插件目录入口设计：列出 + 按 ID 取插件

- 日期：2026-08-01
- 范围：`file_uploader_core`（入口/目录）、`file_uploader_plugins`（显式清单）、`file_uploader_plugins/build.rs`（资源路径注入）
- 状态：待评审

## 1. 背景与动机

当前进程内插件存在两处使用侧的痛点：

1. **无统一入口**。`file_uploader_core/src/main.rs` 中要手动 `Box::new(DefaultInputHandler)`、手动拼 `target/debug/resources/<phase>/<name>` 资源路径、再 `UploadPluginInfo::new_in_process` 一个个构造。上层应用（CLI / Web 后台）想「列出所有内置插件做展示」或「按 ID 取插件调用」没有现成 API。
2. **无发现机制**。Rust 没有运行时反射，进程内插件是编译期具体类型（`DefaultInputHandler`、`UploadFileValidator`，后续还有在途的 `common_uploader`、`common_output`），`core` 无法自行枚举。

进程内插件数量正在增长，需要一套**显式、稳定、面向上层**的目录入口。

## 2. 目标与非目标

### 目标

- 在 `file_uploader_core` 提供「列出所有进程内插件信息」+「按 ID 取插件实现对象」两个入口能力，供上层应用一行调用。
- 以**显式清单**作为发现机制（单一注册点，新增插件改一处）。
- 解决 `main.rs` 硬编码 `target/debug` 资源路径的问题。

### 非目标（YAGNI）

- 不改 `UploadPlugin` / `UploadPluginInfo` / `LazyPluginSlot` 既有语义（pipeline 执行链路零改动）。
- 不引入 `inventory` 等自动注册依赖（当前插件规模不需要去中心化注册）。
- 不把 dylib 插件纳入本目录（本入口只面向进程内插件；dylib 仍走 `UploadPluginInfo::new_from_dylib_path`）。
- 不在 `get` 时自动调 `on_load`（生命周期交调用方显式管理）。
- 不做插件依赖关系编排（仍由 `UploadPluginRegistryTable` 负责）。

## 3. 关键决策（brainstorming 已确认）

| 决策点 | 结论 |
|--------|------|
| 使用场景 | 供上层应用（CLI / Web 后台）调用：展示插件、拉配置 schema、按 ID 取插件 execute 或组装 pipeline |
| 发现机制 | **显式清单**：`file_uploader_plugins::list_in_process_plugins() -> &'static [InProcessEntry]` |
| `list` 返回类型 | 轻量概要 DTO `PluginInfoSummary`（id + meta + config + readme_path，**不可 execute**，纯展示） |
| `get(id)` 返回类型 | `Arc<dyn UploadPlugin>`（**不经 `UploadPluginInfo` 包装**，直接给具体插件实现） |
| 实例生命周期 | **单例复用**（per-id `OnceLock` 缓存）；但 **`on_load`/`on_unload` 由调用方显式管理**，catalog 不自动调 |
| 入口形态 | **方案 C**：以显式 `InProcessPluginCatalog` struct 为核心 + 叠加全局便捷函数 |
| ID 规则 | 由 catalog 加载 `meta.json` 后派生 `in_process_{phase:?}_{name}`，**清单不手写 id**（单一真相来源 = meta） |
| 资源加载失败 | **硬失败**（任一插件资源缺失 → `load_*` 返回 `Err`，早暴露） |

## 4. 架构总览

```
file_uploader_plugins                        core::pipeline::in_process_catalog
┌────────────────────────────┐              ┌────────────────────────────────────────┐
│ pub struct InProcessEntry  │  &'static    │  pub struct PluginInfoSummary {         │
│   resource_subdir: &str    │ ◄─────────── │    id, meta, config, readme_path        │
│   factory: fn() ->         │   清单        │  }                                     │
│     Arc<dyn UploadPlugin>  │              │                                        │
│                            │              │  pub struct InProcessPluginCatalog      │
│ pub fn list_in_process_    │              │   load_default() / load_from(path)     │
│   plugins() -> &[Entry]    │              │   list()  -> &[PluginInfoSummary]      │
│                            │              │   get(id) -> Option<Arc<dyn           │
│ pub fn resources_root()    │  资源根路径  │            UploadPlugin>>             │
│   -> env!("…_RESOURCES…")  │ ◄─────────── │                                        │
└────────────────────────────┘              │  // 全局便捷（OnceLock 单例）           │
   build.rs:                                 │  list_in_process_plugins() -> Result   │
   cargo:rustc-env=                          │  get_in_process_plugin(id)   -> Result │
     FILE_UPLOADER_RESOURCES_DIR             └────────────────────────────────────────┘
```

**依赖方向**：`core` → `file_uploader_plugins`（Cargo.toml 已存在），无新依赖。

**模块布局**：
- `file_uploader_plugins/src/lib.rs`：新增 `InProcessEntry` + `list_in_process_plugins()` + `resources_root()`
- `file_uploader_plugins/build.rs`：追加一行 `cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR=…`
- `file_uploader_core/src/pipeline/in_process_catalog.rs`：新增（DTO + Catalog + 全局函数）
- `file_uploader_core/src/pipeline.rs`：加 `pub mod in_process_catalog;`
- `file_uploader_core/src/lib.rs`：重导出，让上层 `file_uploader_core::list_in_process_plugins()` 一行可达

## 5. 详细设计

### 5.1 清单层（`file_uploader_plugins`）

```rust
use std::sync::Arc;
use file_uploader_sdk::models::interface::UploadPlugin;

/// 单个进程内插件的注册条目（编译期固定）
pub struct InProcessEntry {
    /// 资源目录相对 resources 根的子路径，如 "input/default_input_handler"
    pub resource_subdir: &'static str,
    /// 插件实例工厂（每次调用产新实例；由 catalog 内部 OnceLock 单例化）
    pub factory: fn() -> Arc<dyn UploadPlugin>,
}

/// 所有内置进程内插件的显式清单
pub fn list_in_process_plugins() -> &'static [InProcessEntry] {
    &[
        InProcessEntry {
            resource_subdir: "input/default_input_handler",
            factory: || Arc::new(input::default_input_handler::DefaultInputHandler),
        },
        InProcessEntry {
            resource_subdir: "pre/upload_file_validator",
            factory: || Arc::new(pre_upload::upload_file_validator::UploadFileValidator),
        },
        // 后续 common_uploader / common_output 落地后在此追加
    ]
}
```

> 新增插件只需在清单末尾追加一条 —— 单一注册点。

### 5.2 资源路径（解决 `env!` 不跨 crate 传递）

`cargo:rustc-env` 只对当前 crate 生效。`file_uploader_plugins/build.rs` 已把 `resources/` 复制到 `target/<profile>/resources`，新增一行注入路径：

```rust
println!(
    "cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR={}",
    target_dir.join("resources").display()
);
```

由 `file_uploader_plugins`（自身 crate 内 `env!` 求值合法）暴露访问器，供 `core` 运行时读取：

```rust
pub fn resources_root() -> &'static str {
    env!("FILE_UPLOADER_RESOURCES_DIR")
}
```

`core` 的 `load_default()` 调 `file_uploader_plugins::resources_root()` 得根目录，拼 `resource_subdir` 得每个插件资源目录。

### 5.3 概要 DTO（`core`）

```rust
use serde::Serialize;
use std::sync::Arc;
use crate::pipeline::plugin::{PluginMeta, PluginConfigInfo};

/// 进程内插件的展示信息（不可 execute，纯展示）
#[derive(Serialize, Clone)]
pub struct PluginInfoSummary {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    pub readme_path: Option<String>,
}
```

### 5.4 Catalog struct（`core`）

```rust
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use file_uploader_plugins::{list_in_process_plugins, resources_root, InProcessEntry};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::interface::UploadPlugin;
use crate::pipeline::plugin::PluginResource;

pub struct InProcessPluginCatalog {
    summaries: Vec<PluginInfoSummary>,
    instances: HashMap<String, OnceLock<Arc<dyn UploadPlugin>>>,
    factories: HashMap<String, fn() -> Arc<dyn UploadPlugin>>,
}

impl InProcessPluginCatalog {
    /// 用编译期 env! 默认资源根目录加载
    pub fn load_default() -> Result<Self, UploadError> {
        Self::load_from(Path::new(resources_root()))
    }

    /// 自定义资源根目录（测试/定制场景）
    pub fn load_from(resources_root: &Path) -> Result<Self, UploadError> {
        let entries: &[InProcessEntry] = list_in_process_plugins();
        let mut summaries = Vec::with_capacity(entries.len());
        let mut instances = HashMap::new();
        let mut factories = HashMap::new();

        for entry in entries {
            let dir = resources_root.join(entry.resource_subdir);
            let resource = PluginResource::load(&dir)?;        // 硬失败
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

    /// 所有进程内插件概要（不触发插件加载、不调 on_load）
    pub fn list(&self) -> &[PluginInfoSummary] {
        &self.summaries
    }

    /// 按 id 取插件实现对象（单例复用；on_load 由调用方显式管理）
    pub fn get(&self, id: &str) -> Option<Arc<dyn UploadPlugin>> {
        let cell = self.instances.get(id)?;
        let factory = *self.factories.get(id)?;
        let arc = cell.get_or_init(factory);   // 首次调用执行工厂，之后复用
        Some(arc.clone())
    }
}
```

> `get` 命中后返回的 `Arc<dyn UploadPlugin>` 跨调用指针相等（单例）。`OnceLock::get_or_init` 保证并发安全且工厂只跑一次。

### 5.5 全局便捷函数（`core` 顶层重导出）

```rust
static CATALOG: OnceLock<InProcessPluginCatalog> = OnceLock::new();

fn ensure_catalog() -> Result<&'static InProcessPluginCatalog, UploadError> {
    if let Some(c) = CATALOG.get() {
        return Ok(c);
    }
    let c = InProcessPluginCatalog::load_default()?;   // 失败则不 set，下次重试
    let _ = CATALOG.set(c);                             // 竞态由 OnceLock 收敛
    Ok(CATALOG.get().expect("just set"))
}

pub fn list_in_process_plugins() -> Result<&'static [PluginInfoSummary], UploadError> {
    Ok(ensure_catalog()?.list())
}

pub fn get_in_process_plugin(id: &str) -> Result<Option<Arc<dyn UploadPlugin>>, UploadError> {
    Ok(ensure_catalog()?.get(id))
}
```

`lib.rs` 重导出：
```rust
pub use pipeline::in_process_catalog::{
    get_in_process_plugin, list_in_process_plugins,
    InProcessPluginCatalog, PluginInfoSummary,
};
```

上层调用：
```rust
// 列出
for p in file_uploader_core::list_in_process_plugins()? {
    println!("{}: {} ({:?})", p.id, p.meta.title, p.meta.phase);
}
// 按 ID 取
let plugin = file_uploader_core::get_in_process_plugin("in_process_Input_default_input_handler")?;
```

### 5.6 ID 规则

`id = format!("in_process_{:?}_{}", meta.phase, meta.name)`，与现有 `UploadPluginInfo::new_in_process` 完全一致（`{:?}` 取 `UploadPhase` 的 Debug，如 `Input` / `PreUpload`）。

例：
- `default_input_handler` → `in_process_Input_default_input_handler`
- `upload_file_validator` → `in_process_PreUpload_upload_file_validator`

> 派生而非手写：id 的单一真相来源是 `meta.json`（phase + name），杜绝清单与资源不一致。

### 5.7 错误处理

| 场景 | 行为 |
|------|------|
| `load_*` 时某插件 `meta.json` 缺失 / 解析失败 | 返回 `Err(UploadError::PluginLoadError(..))`（硬失败） |
| `get("未知 id")` | 返回 `None`（不报错） |
| 全局函数首次 `load_default()` 失败 | `OnceLock` 不 set，`Err` 透传；下次调用重新尝试 |

## 6. 测试策略（TDD）

**单元测试**（`load_from` + 临时资源目录，不依赖 target）：
1. `list` 数量与清单条目数一致
2. id 派生正确（`in_process_Input_default_input_handler`）
3. 同 id 两次 `get` 返回**同一 `Arc`**（`Arc::ptr_eq` 为真 → 单例）
4. `get("不存在")` 返回 `None`
5. `list` / `get` 后插件**未触发 `on_load`**（用带计数器的 mock 插件验证 load_count == 0）

**集成测试**（依赖 `target/debug/resources` 存在，沿用现有 `skip if !exists` 模式）：
6. `load_default()` 能加载到真实的 `DefaultInputHandler` / `UploadFileValidator`
7. 取到的插件能正常 `execute`（构造空 ctx，不 panic）

## 7. 改动面（文件清单）

新增：
- `file_uploader_core/src/pipeline/in_process_catalog.rs`（DTO + Catalog + 全局函数 + 测试）

修改：
- `file_uploader_plugins/src/lib.rs`：新增 `InProcessEntry` + `list_in_process_plugins()` + `resources_root()`
- `file_uploader_plugins/build.rs`：追加 `cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR=…`
- `file_uploader_core/src/pipeline.rs`：加 `pub mod in_process_catalog;`
- `file_uploader_core/src/lib.rs`：重导出四个符号
- `docs/references/plugin-specification.md`：新增 §1.7 catalog 入口、§2 ID 生成交叉引用、§8.1 env! 注入说明
- `.agents/skills/designing-in-process-plugins/SKILL.md`：必备文件清单补登记项、新增「登记到清单」步骤、快速检查清单补一项

> `main.rs` 不强制改动（可后续用它替换手动实例化，作为示范迁移，但不在本次范围硬性要求）。

## 8. 风险与取舍

| 风险 | 对策 |
|------|------|
| 清单与实际插件类型脱节（忘登记新插件） | 清单是唯一注册点，CI 集成测试会暴露遗漏；新增插件时 skill 文档提醒同步清单 |
| 全局 `OnceLock` 单例的资源路径编译期固定 | 已提供 `InProcessPluginCatalog::load_from(path)` 给测试/定制场景绕开全局单例 |
| `get` 不自动 `on_load`，调用方忘记调导致插件未初始化 | 文档明确契约；与 `LazyPluginSlot` 区分开（后者 execute 时自动 load，本入口面向"取对象"，生命周期归调用方） |
| 派生 id 依赖 `UploadPhase` 的 Debug 格式 | Debug 是标准派生，稳定；且与既有 `UploadPluginInfo::new_in_process` 同源，未来若改 id 规则需两处同步（可接受） |

## 9. 规范同步（specification.md / SKILL.md）

清单机制须沉淀到长期文档，与代码一同交付（呼应 AGENTS.md「规范变更需同步规范文档与 skill」）。

### 9.1 `docs/references/plugin-specification.md`

**① 新增 §1.7「进程内插件目录入口（catalog）」**（插入 §1.6 之后）

内容要点：
- 进程内插件经**显式清单** `file_uploader_plugins::list_in_process_plugins() -> &'static [InProcessEntry]` 登记（新增插件的唯一注册点）
- `InProcessEntry { resource_subdir: &'static str, factory: fn() -> Arc<dyn UploadPlugin> }`
- `core::pipeline::in_process_catalog` 提供：
  - `InProcessPluginCatalog::load_default()` / `load_from(path)`
  - `list() -> &[PluginInfoSummary]`（概要：id + meta + config + readme_path，**不可 execute**）
  - `get(id) -> Option<Arc<dyn UploadPlugin>>`（单例复用；**`on_load` 由调用方显式管理**）
- 全局便捷函数 `file_uploader_core::list_in_process_plugins()` / `get_in_process_plugin(id)`
- ID 由 `meta.json` 派生（见 §2「插件 ID 生成」）
- 一段简短调用示例

**② §2「插件 ID 生成」节末追加一行交叉引用**：

> 进程内插件目录入口（§1.7）的 `list` / `get` 沿用此 ID 规则作为查找 key。

**③ §8.1「进程内插件」补一段**：

`build.rs` 另注入 `cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR=<target>/<profile>/resources`，
由 `file_uploader_plugins::resources_root() -> &'static str` 暴露，
供 `InProcessPluginCatalog::load_default()` 定位资源根（解决 `main.rs` 硬编码 `target/debug` 的旧问题）。

### 9.2 `.agents/skills/designing-in-process-plugins/SKILL.md`

**① 必备文件清单表补一行**：

| 文件 | 必需 | 作用 |
|---|---|---|
| `file_uploader_plugins/src/lib.rs` | 新增插件必须 | 在 `list_in_process_plugins()` 登记 `InProcessEntry` |

**② 实现步骤新增一步「登记到进程内插件清单」**（置于「8. 模块注册」之后，原 9–11 顺延）：

在 `list_in_process_plugins()` 末尾追加一条：

```rust
InProcessEntry {
    resource_subdir: "pre/size_limiter",   // <phase 短名>/<name>，与资源目录一致
    factory: || Arc::new(SizeLimiter),
},
```

> 不登记则不会被 `InProcessPluginCatalog` 发现，上层「列出 / 按 ID 取」拿不到该插件。

**③ 快速检查清单「代码」段补一项**：

- [ ] 已在 `list_in_process_plugins()` 追加 `InProcessEntry`（`resource_subdir` 与资源目录路径一致）

## 10. 未来扩展

- 清单可演进为带元信息（优先级默认值、是否默认启用）以辅助 `UploadPluginRegistryTable` 自动组装。
- 若进程内插件数量显著增长，可再评估迁移到 `inventory` 去中心化注册。
- `PluginInfoSummary` 可加 `validate_config_declarative` 便捷方法，供上层在校验用户配置时免手写 schema 引用。
