# 插件配置表达重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将进程内与 dylib 两种插件的配置表达统一到 `resources/` 目录化格式（`meta.json` + `config.json`，表单驱动 schema），并抽公共加载器消除重复。

**Architecture:** 在 `file_uploader_core/src/pipeline/plugin.rs` 新增 schema 类型族（`PluginConfigInfo`/`PluginConfigItem`/`PluginFormSpec`/`PluginValueOption`）与公共加载器 `PluginResource::load(dir)`；改造 `UploadPluginInfo` 两入口共用加载器、统一 id 策略；目录化迁移两类插件资源文件；改造 `build.rs`；同步 `AGENTS.md`/`README.md`。

**Tech Stack:** Rust edition 2024 / serde / serde_json / thiserror / libloading / stabby

**Spec:** `docs/superpowers/specs/2026-06-16-plugin-config-design.md`

---

## File Structure

| 文件 | 责任 | 操作 |
|---|---|---|
| `file_uploader_sdk/src/models/enums.rs` | 给 `UploadConfigType` 补 `Clone` | 修改 |
| `file_uploader_core/src/pipeline/plugin.rs` | schema 类型族 + `PluginResource` 加载器 + `UploadPluginInfo` 改造；退役旧 `PluginConfig` | 修改 |
| `file_uploader_core/src/pipeline/registry.rs` | 适配 `LazySlotSource` 字段更名、`UploadPluginInfo.config` 字段（含测试夹具） | 修改 |
| `file_uploader_core/src/main.rs` | 入口调用路径更新 | 修改 |
| `file_uploader_plugins/resources/pre/file_type_filter/config.json` | 新 schema | 改写 |
| `file_uploader_plugins/{pre_upload,upload,post_upload}_plugins.json` | 旧聚合文件 | 删除 |
| `file_uploader_plugins/build.rs` | 递归复制 `resources/` | 改写 |
| `uploader_example_plugin/{meta.json,config.json,plugin.id}` | 新建/改写/占位 | 新建/改写 |
| `uploader_example_plugin/build.rs` | 复制三件到 dylib 同目录 | 改写 |
| `AGENTS.md` / `README.md` | 文档同步 | 修改 |

---

## Task 1: schema 数据模型 + 反序列化测试

**Files:**
- Modify: `file_uploader_sdk/src/models/enums.rs`（`UploadConfigType` 加 `Clone`）
- Modify: `file_uploader_core/src/pipeline/plugin.rs`（新增类型族，**暂不删旧 `PluginConfig`、暂不动 `UploadPluginInfo`**）

- [ ] **Step 1: 给 `UploadConfigType` 补 `Clone`**

`file_uploader_sdk/src/models/enums.rs:81-89`，把：
```rust
#[derive(Serialize, Deserialize, Debug)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadConfigType {
```
改为：
```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadConfigType {
```

- [ ] **Step 2: 先写失败测试**

在 `file_uploader_core/src/pipeline/plugin.rs` 末尾的 `#[cfg(test)] mod tests` 内追加：
```rust
    #[test]
    fn test_plugin_config_info_select_text_deserialize() {
        let json = r#"{
            "access": {},
            "params": [
                {
                    "key": "pass_type",
                    "title": "允许类型",
                    "description": "为空全部允许",
                    "config_type": "Custom",
                    "default_value": [],
                    "form": {
                        "type": "select",
                        "options": [{"label":"图片","value":"image"}],
                        "multiple": true,
                        "allow_custom": true
                    }
                },
                {
                    "key": "token",
                    "title": "凭证",
                    "description": "访问凭证",
                    "config_type": "Default",
                    "default_value": "",
                    "form": { "type": "text", "secret": true }
                }
            ]
        }"#;
        let info: super::PluginConfigInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.params.len(), 2);
        assert!(matches!(info.params[0].form, super::PluginFormSpec::Select { multiple: true, .. }));
        assert!(matches!(info.params[1].form, super::PluginFormSpec::Text { secret: true }));
    }

    #[test]
    fn test_plugin_form_spec_serde_default_omitted() {
        // 省略所有可选字段
        let json = r#"{ "type": "text" }"#;
        let form: super::PluginFormSpec = serde_json::from_str(json).unwrap();
        assert!(matches!(form, super::PluginFormSpec::Text { secret: false }));

        let json2 = r#"{ "type": "select" }"#;
        let form2: super::PluginFormSpec = serde_json::from_str(json2).unwrap();
        assert!(matches!(form2, super::PluginFormSpec::Select { options, multiple: false, allow_custom: false } if options.is_empty()));
    }

    #[test]
    fn test_plugin_config_info_default_empty() {
        let d = super::PluginConfigInfo::default();
        assert!(d.params.is_empty());
        assert!(d.access.is_object());
    }
```

- [ ] **Step 3: 运行测试确认失败（类型未定义，编译错误）**

Run: `cargo test -p file_uploader_core --no-run`
Expected: 编译错误 `cannot find type PluginConfigInfo` 等

- [ ] **Step 4: 新增类型族**

在 `file_uploader_core/src/pipeline/plugin.rs` 中、`PluginMeta` 定义之后、旧 `PluginConfig` 之前，插入：
```rust
/// **插件配置文件容器**（对应 config.json）
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginConfigInfo {
    /// 访问控制（reserved，纯透传）
    #[serde(default = "empty_object")]
    pub access: Value,
    /// 配置项列表
    #[serde(default)]
    pub params: Vec<PluginConfigItem>,
}

fn empty_object() -> Value {
    serde_json::json!({})
}

/// **单个配置项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginConfigItem {
    pub key: String,
    pub title: String,
    pub description: String,
    pub config_type: UploadConfigType,
    pub default_value: Value,
    pub form: PluginFormSpec,
}

/// **表单控件描述**（value_type + value_config 合并表达）
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginFormSpec {
    Text {
        #[serde(default)]
        secret: bool,
    },
    Select {
        #[serde(default)]
        options: Vec<PluginValueOption>,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        allow_custom: bool,
    },
}

/// **候选项**
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginValueOption {
    pub label: String,
    pub value: Value,
}
```

> `UploadConfigType` 已在文件顶部 import（`use file_uploader_sdk::models::enums::{PluginLogLevel, UploadConfigType, UploadPhase};`），无需新 import。`Value`、`Serialize`、`Deserialize` 已 import。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p file_uploader_core test_plugin_config_info test_plugin_form_spec`
Expected: PASS（3 个测试全绿）

- [ ] **Step 6: Commit**

```bash
git add file_uploader_sdk/src/models/enums.rs file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat(core): add form-driven plugin config schema types"
```

---

## Task 2: `PluginResource` 公共加载器 + 测试

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs`

- [ ] **Step 1: 先写失败测试**

在 `plugin.rs` 的 `#[cfg(test)] mod tests` 内追加（使用 `tempfile` 会引入依赖，改用构建期已复制到 `target/` 的真实资源目录；本测试依赖 Task 5 完成后的 `resources/pre/file_type_filter`，故此测试在 Task 5 后才会真正通过——此处先写好断言逻辑，运行时机见 Step 4 说明）：
```rust
    fn target_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()      // workspace root
            .join("target/debug")
    }

    #[test]
    fn test_plugin_resource_load_success() {
        let dir = target_dir().join("resources/pre/file_type_filter");
        if !dir.exists() {
            // Task 5 前资源尚未复制到 target；跳过避免噪音
            eprintln!("skip: {} not ready yet", dir.display());
            return;
        }
        let r = super::PluginResource::load(&dir).unwrap();
        assert_eq!(r.meta.name, "file_type_filter");
        assert!(!r.config.params.is_empty());
    }

    #[test]
    fn test_plugin_resource_load_missing_meta() {
        let dir = target_dir().join("resources/__nonexistent__");
        let r = super::PluginResource::load(&dir);
        assert!(r.is_err());
    }
```

- [ ] **Step 2: 运行确认失败（`PluginResource` 未定义）**

Run: `cargo test -p file_uploader_core --no-run`
Expected: 编译错误 `cannot find type PluginResource`

- [ ] **Step 3: 实现 `PluginResource`**

在 `plugin.rs`、紧接 Task 1 新增类型族之后，插入：
```rust
/// **插件资源（meta + config 加载结果）**
pub struct PluginResource {
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
}

impl PluginResource {
    /// 从插件资源目录加载：meta.json 必读，config.json 选读（缺失→空容器）。
    pub fn load(dir: &Path) -> Result<Self, UploadError> {
        let meta_path = dir.join("meta.json");
        let meta_file = File::open(&meta_path).map_err(|e| {
            UploadError::PluginLoadError(format!(
                "Failed to open meta file {}: {}",
                meta_path.display(),
                e
            ))
        })?;
        let meta: PluginMeta = serde_json::from_reader(meta_file)?;

        let config_path = dir.join("config.json");
        let config = match File::open(&config_path) {
            Ok(f) => Arc::new(serde_json::from_reader(f)?),
            Err(_) => Arc::new(PluginConfigInfo::default()),
        };

        Ok(PluginResource { meta: Arc::new(meta), config })
    }
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p file_uploader_core test_plugin_resource_load`
Expected: `test_plugin_resource_load_missing_meta` PASS；`test_plugin_resource_load_success` 此时因资源未复制到 target 会走 skip 分支（Task 5 后自动生效）。

- [ ] **Step 5: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat(core): add PluginResource loader for meta+config"
```

---

## Task 3: 改造 `UploadPluginInfo` + 退役旧 `PluginConfig` + 适配 registry 测试

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs`
- Modify: `file_uploader_core/src/pipeline/registry.rs`（测试夹具）

- [ ] **Step 1: 改 `LazySlotSource` 字段名 `config_path` → `resource_dir`**

`plugin.rs:81-90`：
```rust
pub(crate) enum LazySlotSource {
    InProcess {
        resource_dir: String,
        plugin: Arc<dyn UploadPlugin>,
    },
    Dylib {
        resource_dir: String,
        dylib_path: String,
    },
}
```

- [ ] **Step 2: 改 `UploadPluginInfo` 字段**

`plugin.rs:196-209`，把 `default_config` 字段替换为 `config`：
```rust
#[derive(Serialize)]
pub struct UploadPluginInfo {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    pub path: String,
    #[serde(skip)]
    pub slot: LazyPluginSlot,
}
```

- [ ] **Step 3: 重写 `new_in_process`**

`plugin.rs:211-255` 整个 `new_in_process` 替换为：
```rust
    pub fn new_in_process(
        resource_dir: &str,
        plugin: Box<dyn UploadPlugin>,
    ) -> Result<UploadPluginInfo, UploadError> {
        let resource = PluginResource::load(Path::new(resource_dir))?;
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: resource_dir.to_string(),
                plugin: Arc::from(plugin),
            },
            inner: OnceLock::new(),
        };
        let id = format!(
            "in_process_{:?}_{}",
            resource.meta.phase, resource.meta.name
        );
        Ok(UploadPluginInfo {
            id,
            meta: resource.meta,
            config: resource.config,
            path: resource_dir.to_string(),
            slot,
        })
    }
```

- [ ] **Step 4: 重写 `new_from_dylib_path`**

`plugin.rs:265-324` 整个 `new_from_dylib_path` 替换为：
```rust
    pub fn new_from_dylib_path(dylib_path: &str) -> Result<UploadPluginInfo, UploadError> {
        let dylib_path_obj = Path::new(dylib_path);
        let resource_dir = dylib_path_obj.parent().ok_or_else(|| {
            UploadError::PluginLoadError("Invalid dylib path: no parent directory".to_string())
        })?;

        let resource = PluginResource::load(resource_dir)?;

        let id_path = resource_dir.join("plugin.id");
        let id = std::fs::read_to_string(&id_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                UploadError::PluginLoadError(format!(
                    "Failed to read plugin.id at {}: {}",
                    id_path.display(),
                    e
                ))
            })?;

        let slot = LazyPluginSlot {
            source: LazySlotSource::Dylib {
                resource_dir: resource_dir.display().to_string(),
                dylib_path: dylib_path.to_string(),
            },
            inner: OnceLock::new(),
        };

        Ok(UploadPluginInfo {
            id,
            meta: resource.meta,
            config: resource.config,
            path: dylib_path.to_string(),
            slot,
        })
    }
```

- [ ] **Step 5: 替换 getter，删除旧 `PluginConfig`**

`plugin.rs:326-344` 区段，把 `execute / on_load / get_id / get_default_config / get_meta` 替换为（仅改 `get_default_config` → `get_config`，其余原样保留）：
```rust
    pub fn execute(&self, context: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        self.slot.execute(context)
    }

    pub fn on_load(&self) -> Result<(), UploadError> {
        self.slot.on_load()
    }

    pub fn get_id(&self) -> String {
        self.id.clone()
    }

    pub fn get_config(&self) -> Arc<PluginConfigInfo> {
        self.config.clone()
    }

    pub fn get_meta(&self) -> Arc<PluginMeta> {
        self.meta.clone()
    }
```

然后**删除**旧的 `PluginConfig` 结构体定义（`plugin.rs:182-193`，`pub struct PluginConfig { ... }`）。

- [ ] **Step 6: 修复 `plugin.rs` 顶部测试里用到旧 id 逻辑的断言**

`plugin.rs` 末尾现有 `test_new_from_dylib_path_success` / `test_new_from_dylib_path_invalid_path`。更新 `test_new_from_dylib_path_invalid_path` 的错误信息匹配（路径无效现在仍报 "no parent directory"，或 dylib 在根目录时报 config/meta 错误），保持原断言风格：
```rust
    #[test]
    fn test_new_from_dylib_path_invalid_path() {
        let result = UploadPluginInfo::new_from_dylib_path("/");
        assert!(result.is_err());
        match result {
            Err(UploadError::PluginLoadError(msg)) => {
                assert!(
                    msg.contains("no parent directory")
                        || msg.contains("Failed to open meta file")
                        || msg.contains("Failed to read plugin.id")
                );
            }
            _ => panic!("Expected PluginLoadError"),
        }
    }
```
> `test_new_from_dylib_path_success` 依赖真实 dylib + meta/config/plugin.id，将在 Task 6 后通过；本步保留即可。

- [ ] **Step 7: 适配 `registry.rs` 测试夹具**

`registry.rs` 测试中所有 `LazySlotSource::InProcess { config_path: ..., plugin }` 改为 `LazySlotSource::InProcess { resource_dir: ..., plugin }`；所有 `UploadPluginInfo { ..., default_config: None, ... }` 改为 `UploadPluginInfo { ..., config: std::sync::Arc::new(super::super::plugin::PluginConfigInfo::default()), ... }`。

具体位置（按行号近似，实际以 grep 为准）：
- `registry.rs:318` `use crate::pipeline::plugin::{LazyPluginSlot, LazySlotSource, PluginMeta};` → 追加 `PluginConfigInfo`：
  ```rust
  use crate::pipeline::plugin::{LazyPluginSlot, LazySlotSource, PluginConfigInfo, PluginMeta};
  ```
- 所有 `config_path: "/test/path".to_string()` → `resource_dir: "/test/path".to_string()`
- 所有 `default_config: None,` → `config: Arc::new(PluginConfigInfo::default()),`

可用以下命令定位后逐一改：
```bash
rg -n 'config_path:|default_config:' file_uploader_core/src/pipeline/registry.rs
```

- [ ] **Step 8: 编译并跑 core 全量测试**

Run: `cargo build -p file_uploader_core`
Expected: 编译通过（无 `PluginConfig` / `default_config` / `config_path` 残留引用）

Run: `cargo test -p file_uploader_core`
Expected: 除依赖真实 dylib/resource 的测试可能 skip/fail 外，其余通过。

- [ ] **Step 9: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs file_uploader_core/src/pipeline/registry.rs
git commit -m "refactor(core): rework UploadPluginInfo around PluginResource, retire PluginConfig"
```

---

## Task 4: `main.rs` 入口调用更新

**Files:**
- Modify: `file_uploader_core/src/main.rs`

- [ ] **Step 1: 更新进程内插件加载路径**

`main.rs:27-29`：
```rust
    let Ok(plugin) =
        UploadPluginInfo::new_in_process("./resources/pre/file_type_filter", Box::new(FileTypeFilter))
    else {
        error!("Plugin load error");
        return;
    };
```

- [ ] **Step 2: 编译**

Run: `cargo build -p file_uploader_core`
Expected: PASS（运行验证留待 Task 5 后）

- [ ] **Step 3: Commit**

```bash
git add file_uploader_core/src/main.rs
git commit -m "chore(core): point in-process example at resources dir"
```

---

## Task 5: 进程内资源文件迁移 + `file_uploader_plugins/build.rs` 递归复制

**Files:**
- Delete: `file_uploader_plugins/pre_upload_plugins.json`
- Delete: `file_uploader_plugins/upload_plugins.json`
- Delete: `file_uploader_plugins/post_upload_plugins.json`
- Modify: `file_uploader_plugins/resources/pre/file_type_filter/config.json`
- Modify: `file_uploader_plugins/build.rs`

- [ ] **Step 1: 删除三个聚合 json**

```bash
git rm file_uploader_plugins/pre_upload_plugins.json file_uploader_plugins/upload_plugins.json file_uploader_plugins/post_upload_plugins.json
```

- [ ] **Step 2: 改写 `config.json` 为新 schema**

`file_uploader_plugins/resources/pre/file_type_filter/config.json` 全文替换为：
```json
{
  "access": {},
  "params": [
    {
      "key": "pass_type",
      "title": "允许通过的文件类型",
      "description": "为空表示全部允许，支持匹配占位符",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true
      }
    },
    {
      "key": "reject_type",
      "title": "拦截的文件类型",
      "description": "为空表示全部允许通过，支持匹配占位符",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true
      }
    }
  ]
}
```

- [ ] **Step 3: 核对 `meta.json`**

`file_uploader_plugins/resources/pre/file_type_filter/meta.json` 已含 `name/title/description/version/author/phase`，无需改动。确认字段齐全即可。

- [ ] **Step 4: 改写 `build.rs` 为递归复制 `resources/`**

`file_uploader_plugins/build.rs` 全文替换为：
```rust
use std::fs;
use std::path::Path;

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path);
        } else {
            fs::copy(&path, &dst_path)
                .unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", path, dst_path, e));
        }
    }
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // OUT_DIR 的 parent×3 = target/<profile>
    let target_dir = std::path::PathBuf::from(&out_dir)
        .parent().unwrap()
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();

    eprintln!("target_dir = {}", target_dir.display());

    let src = std::path::PathBuf::from(&manifest_dir).join("resources");
    let dst = target_dir.join("resources");

    if src.exists() {
        copy_dir_recursive(&src, &dst);
    } else {
        eprintln!("warn: resources dir not found at {}", src.display());
    }

    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=build.rs");
}
```

- [ ] **Step 5: 构建并运行，验证进程内插件加载新格式**

Run: `cargo build -p file_uploader_plugins && cargo run -p file_uploader_core`
Expected: 日志输出 `Plugin loaded: in_process_PreUpload_file_type_filter`，执行成功（`Plugin execute result` 为 Success）。

- [ ] **Step 6: 回头验证 Task 2 的 `test_plugin_resource_load_success` 已生效**

Run: `cargo test -p file_uploader_core test_plugin_resource_load`
Expected: 两个测试均 PASS（不再 skip）。

- [ ] **Step 7: Commit**

```bash
git add file_uploader_plugins/
git commit -m "refactor(plugins): migrate in-process plugins to resources dir layout"
```

---

## Task 6: dylib 资源文件迁移 + `uploader_example_plugin/build.rs` 复制三件

**Files:**
- Create: `uploader_example_plugin/meta.json`
- Modify: `uploader_example_plugin/config.json`（扁平旧 → 新 schema）
- Create: `uploader_example_plugin/plugin.id`
- Modify: `uploader_example_plugin/build.rs`

- [ ] **Step 1: 新建 `meta.json`**（从旧 `config.json` 提取元数据）

`uploader_example_plugin/meta.json`：
```json
{
  "name": "uploader_test_example_plugin",
  "title": "动态加载测试插件",
  "description": "动态加载测试插件",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "PreUpload"
}
```

- [ ] **Step 2: 改写 `config.json` 为新 schema**

`uploader_example_plugin/config.json` 全文替换为：
```json
{
  "access": {},
  "params": [
    {
      "key": "pass_type",
      "title": "允许上传的文件类型",
      "description": "允许上传的文件类型（为空表示全部允许）",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true
      }
    },
    {
      "key": "reject_type",
      "title": "不允许上传的文件类型",
      "description": "不允许上传的文件类型（为空表示全部允许）",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "multiple": true,
        "allow_custom": true
      }
    }
  ]
}
```

- [ ] **Step 3: 新建占位 `plugin.id`**

`uploader_example_plugin/plugin.id`（CLI 落地前的占位测试值，CLI 接管后覆盖）：
```
dylib_uploader_test_example_plugin_20260616_0001
```

- [ ] **Step 4: 改写 `build.rs` 复制三件到 dylib 同目录**

`uploader_example_plugin/build.rs` 全文替换为：
```rust
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let target_dir = PathBuf::from(&out_dir)
        .parent().unwrap()
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();

    eprintln!("target_dir = {}", target_dir.display());

    for file in ["meta.json", "config.json", "plugin.id"] {
        let src = PathBuf::from(&manifest_dir).join(file);
        let dst = target_dir.join(file);
        fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));
        println!("cargo:rerun-if-changed={}", file);
    }

    println!("cargo:rerun-if-changed=build.rs");
}
```

- [ ] **Step 5: 构建并运行，验证 dylib 插件加载**

Run: `cargo build -p uploader_example_plugin && cargo run -p file_uploader_core`
Expected: 进程内插件与 dylib 插件均加载成功；dylib 段日志 `Dylib plugin loaded: dylib_uploader_test_example_plugin_20260616_0001`（id 来自 `plugin.id`）。

- [ ] **Step 6: 验证 `test_new_from_dylib_path_success`**

Run: `cargo test -p file_uploader_core test_new_from_dylib_path`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add uploader_example_plugin/
git commit -m "refactor(example-plugin): adopt meta+config+plugin.id dir layout"
```

---

## Task 7: 同步 `AGENTS.md`

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: 按设计稿 §8.1 逐项修订**

依 `docs/superpowers/specs/2026-06-16-plugin-config-design.md` §8.1，修订 `AGENTS.md` 中：
1. 项目结构：移除不存在的 `plugin.json`；`file_uploader_plugins/` 增加 `resources/<phase>/<name>/{meta,config}.json`；`plugin.rs` 描述补入 `PluginConfigInfo / PluginConfigItem / PluginFormSpec / PluginResource`。
2. 「插件配置（`PluginConfig`）」段更新为 `PluginConfigItem` + `PluginConfigInfo` + `PluginFormSpec`（`Text`/`Select`）。
3. 「配置文件格式」段：改为统一目录化（`meta.json`+`config.json`，dylib 另含 `plugin.id`），附新 schema 示例。
4. 「插件信息（`UploadPluginInfo`）」段：`new_in_process(resource_dir, plugin)`、`new_from_dylib_path` 读同目录 meta+config+plugin.id；id 策略（进程内 `in_process_{phase}_{name}`、dylib 读 `plugin.id`）。
5. `build.rs` 段：进程内递归复制 `resources/`；dylib 复制三件。
6. 移除 `unknow` typo 相关注述。

> 修订时遵循 AGENTS.md 现有 Markdown 风格与章节层级。

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: sync AGENTS.md with plugin config refactor"
```

---

## Task 8: 同步 `README.md`

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 按设计稿 §8.2 逐项修订**

依 spec §8.2，修订 `README.md`：
1. 特性「插件配置」（约第 12 行）：改为目录化 `meta.json`+`config.json` + 表单驱动 schema（`form`/`text`/`select`）。
2. 项目结构：`file_uploader_plugins/` 增加 `resources/`；`plugin.rs` 描述补入新类型族。
3. 「开发进程内插件」配置示例（约第 147–168 行）：旧嵌套聚合格式 → 目录化 `meta.json` + `config.json`（含 `form:{type,...}`）示例。
4. 「开发动态库插件」段（约第 170–208 行）：补「`.dylib` 同目录需含 `meta.json`+`config.json`+`plugin.id`」说明。
5. 「使用 Pipeline」示例（约第 219 行）：`new_in_process("config.json", ...)` → `new_in_process("./resources/pre/my-plugin", ...)`。

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: sync README.md with plugin config refactor"
```

---

## Task 9: 全量验证与清理

**Files:** 无（仅验证）

- [ ] **Step 1: workspace 全量构建**

Run: `cargo build`
Expected: PASS（4 个 crate 全部编译通过）

- [ ] **Step 2: workspace 全量测试**

Run: `cargo test`
Expected: 全绿（含 Task 2/3/6 的资源加载、id、反序列化测试）

- [ ] **Step 3: 端到端冒烟**

Run: `cargo run -p file_uploader_core`
Expected: 进程内 + dylib 两段均加载成功，id 分别为 `in_process_PreUpload_file_type_filter` 与 `plugin.id` 内容。

- [ ] **Step 4: 残留检查**

```bash
rg -n 'default_config|config_path|PluginConfig\b|unknow' --type rust
rg -n 'pre_upload_plugins.json|upload_plugins.json|post_upload_plugins.json'
```
Expected: 无业务代码残留（测试注释中如出现可酌情清理）。

- [ ] **Step 5: 最终提交（如有清理改动）**

```bash
git add -A
git commit -m "test: verify plugin config refactor end-to-end"
```

---

## Self-Review 记录

- **Spec 覆盖**：§4 数据模型 → Task 1；§5 加载器/入口/id → Task 2/3；§6 迁移/build/测试 → Task 5/6/9；§7 改动清单 → 全部 task；§8 文档同步 → Task 7/8；§9 TODO（CLI）显式不在本计划范围。无遗漏。
- **类型一致性**：`PluginConfigInfo` / `PluginConfigItem` / `PluginFormSpec` / `PluginValueOption` / `PluginResource` 在 Task 1/2 定义，Task 3 的 `UploadPluginInfo.config: Arc<PluginConfigInfo>`、`get_config() -> Arc<PluginConfigInfo>`、registry 夹具 `PluginConfigInfo::default()` 引用一致；`LazySlotSource` 字段更名 `resource_dir` 在 Task 3 Step1/Step3/Step4 与 registry Step7 一致。
- **占位扫描**：`plugin.id` 占位值（Task 6 Step 3）是 spec §9 明确的过渡占位，非实现期遗留 TODO；其余步骤均含完整代码/命令。
