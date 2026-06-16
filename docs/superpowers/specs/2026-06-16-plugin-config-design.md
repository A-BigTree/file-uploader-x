# 插件配置表达重构设计

- 日期：2026-06-16
- 状态：已评审定稿，待制定实现计划
- 适用范围：`file_uploader_sdk` / `file_uploader_core` / `file_uploader_plugins` / `uploader_example_plugin`

## 1. 背景与动机

当前「插件配置」存在多套不一致的表达，技术债集中体现在：

1. **三套配置文件格式并存**
   - 进程内聚合式 `pre_upload_plugins.json`：`{ "<plugin-name>": { meta..., "config": { key: PluginConfig } } }`
   - dylib 扁平式 `config.json`：`{ meta..., "config": {...} }`
   - `resources/pre/file_type_filter/` 下已出现但**未被代码消费**的新格式：`meta.json` + `config.json`（`{ access:{}, params:[{key,config_type,title,description,default_value,value_type,value_config}] }`，字段更丰富）

2. **`default_config`（声明 schema）形同虚设**：注册时 `registry_config: Option<Value>` 是裸 JSON，不校验 key 合法性、不补默认值、不做类型检查。

3. **进程内 / dylib 加载逻辑近乎重复**：`UploadPluginInfo::new_in_process` 与 `new_from_dylib_path` 的 meta+config 解析几乎一致，仅入口不同；另有 `unknow`/`unknown` 的 author fallback typo。

4. **配置项类型表达不足**：`PluginConfig` 只有 `key/config_type/description/default_value`，而 `resources` 新格式已引入 `title/value_type/value_config/access` 等面向「前端表单渲染」的语义。

## 2. 目标与非目标

### 目标
- 以 `resources/` 下那套新格式为**统一目标态**，进程内插件与 dylib 插件各自在自己的模块内按此格式重构。
- 用一个**公共资源加载器**消除两类插件加载入口的重复解析逻辑。
- 让 `PluginConfig`（升级为 `PluginConfigItem`）忠实承载表单驱动的 schema 字段。

### 非目标（明确不做）
- 不实现「插件构建 CLI」及 `plugin.id` 的真实生成算法（记 TODO）。
- 不在 Rust 运行期对 schema 做校验 / 补默认值 —— **纯透传**。
- 不改 ABI：`UploadInputCtxS.config_info` 维持 `SOption<SString>`（JSON 字符串）。
- 不动 `PipelineCallback`、注册表排序、`execute_pipeline` 主体逻辑。
- 不保留旧格式兼容（一刀切迁移）。

## 3. 总体决策（已定约束）

| 维度 | 决策 |
|---|---|
| 目标格式 | 每插件一目录：`meta.json` + `config.json`（`{ access, params:[...] }`） |
| 适用范围 | 进程内 + dylib 各自模块内统一采用 |
| 运行期介入 | 纯透传，不做校验/补默认值 |
| 编排信息 | `priority / status / registry_config` 仍走代码注入 `PluginRegistryInfo` |
| 插件标识权威 | `meta.json` 的 `name`；目录名仅为文件系统约定 |
| 实现路线 | 目录化 + 抽公共资源加载器（方案 A） |

## 4. 数据模型

> 全部位于 `file_uploader_core/src/pipeline/plugin.rs`（**不新建 `resource.rs`**）；不进 SDK、不跨 stabby ABI —— schema 仅宿主侧加载时消费。

### 4.1 `PluginMeta`（不变，与 `meta.json` 对齐）

字段：`name / title / version / description / author: Option<String> / phase: UploadPhase`

### 4.2 `config.json` 的 Rust 容器 —— `PluginConfigInfo`

```rust
pub struct PluginConfigInfo {
    pub access: PluginAccessConfig,      // 权限配置，纯透传
    pub params: Vec<PluginConfigItem>,
}
```

> `PluginConfigInfo` 实现 `Default`（`access = PluginAccessConfig::default()`、`params = []`），用于 `config.json` 缺失时的兜底空容器。

#### 4.2.1 权限配置 `PluginAccessConfig` / `AccessSpec`

```rust
// 权限粒度：开关 或 白名单
#[serde(untagged)]
pub enum AccessSpec {
    Flag(bool),              // true=全开 / false=全关
    Allowlist(Vec<String>),  // 白名单（fs=路径，network=host）
}
// 默认 Deny：Flag(false)

pub struct PluginAccessConfig {
    pub fs_read: AccessSpec,
    pub fs_write: AccessSpec,
    pub network: AccessSpec,
    pub extra: serde_json::Value,  // 预留扩展点
}
```

权限点缺省 = `Flag(false)`（Deny）。纯透传，不做执行/校验逻辑。

### 4.3 单个配置项 —— `PluginConfigItem`（原 `PluginConfig` 升级，旧类型退役）

```rust
pub struct PluginConfigItem {
    pub key: String,
    pub title: String,
    pub description: String,
    pub config_type: UploadConfigType,    // Default | Custom，沿用现有枚举
    pub default_value: serde_json::Value,
    pub form: PluginFormSpec,             // 原 value_type + value_config 合并至此
}
```

### 4.4 表单控件描述 —— `PluginFormSpec`（internally tagged enum）

`value_type` 与 `value_config` 在 JSON 中合并为单一字段 `form`，由 Rust enum 承接。各 variant 字段均 `#[serde(default)]`，配置可省略。

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginFormSpec {
    Text {
        #[serde(default)]
        secret: bool,                     // 是否密码型
    },
    Select {
        #[serde(default)]
        options: Vec<PluginValueOption>,  // 候选项
        #[serde(default)]
        multiple: bool,                   // 是否多选
        #[serde(default)]
        allow_custom: bool,               // 是否允许自定义输入
    },
    // 后续新增控件只需追加 variant
}

#[derive(Serialize, Deserialize)]
pub struct PluginValueOption {
    pub label: String,
    pub value: serde_json::Value,
}
```

### 4.5 `config.json` 示例

```json
{
  "access": { "fs_read": ["/tmp/uploads"], "fs_write": false, "network": false },
  "params": [
    {
      "key": "pass_type",
      "title": "允许通过的文件类型",
      "description": "为空表示全部允许，支持匹配占位符",
      "config_type": "Custom",
      "default_value": [],
      "form": {
        "type": "select",
        "options": [{"label": "图片", "value": "image"}],
        "multiple": true,
        "allow_custom": true
      }
    },
    {
      "key": "token",
      "title": "上传凭证",
      "description": "访问凭证",
      "config_type": "Default",
      "default_value": "",
      "form": { "type": "text", "secret": true }
    }
  ]
}
```

## 5. 资源目录约定与公共加载器

### 5.1 目录形态
- **进程内**：`file_uploader_plugins/resources/<phase>/<plugin_name>/{meta.json, config.json}`（沿用现有 `resources/pre/file_type_filter/`）。
- **dylib**：`.dylib` 产物同目录内放 `{meta.json, config.json, plugin.id}`（取代现扁平 `config.json`）。
- `<phase>` 目录段约定：`pre`→`PreUpload`、`upload`→`Upload`、`post`→`PostUpload`；**不强校验**目录名与 `meta.json.phase` 一致（纯透传），phase 权威仍为 `meta.json`。

### 5.2 公共加载器（位于 `plugin.rs`）

```rust
pub struct PluginResource {
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
}

impl PluginResource {
    pub fn load(dir: &Path) -> Result<Self, UploadError> {
        // meta.json：必读，缺失/解析失败 → Err
        // config.json：选读，缺失 → PluginConfigInfo::default()；
        //               存在但解析失败 → Err（不再吞错）
    }
}
```

### 5.3 加载入口
- `UploadPluginInfo::new_in_process(resource_dir: &str, plugin: Box<dyn UploadPlugin>)`：内部 `PluginResource::load(dir)`。
- `UploadPluginInfo::new_from_dylib_path(dylib_path: &str)`：取 parent 作 resource_dir 调 `PluginResource::load`，并额外读取同目录 `plugin.id`。
- `LazySlotSource` 字段 `config_path` 更名为 `resource_dir`：
  - `InProcess { resource_dir, plugin }`
  - `Dylib { resource_dir, dylib_path }`

### 5.4 `UploadPluginInfo` 字段调整
- `default_config: Option<Arc<HashMap<String, PluginConfig>>>` → **`config: Arc<PluginConfigInfo>`**（恒有值；`config.json` 缺失即空容器）。
- getter `get_default_config()` → `get_config() -> Arc<PluginConfigInfo>`。
- `path` 语义不变（in_process 记 `resource_dir`；dylib 记 dylib 路径）。

### 5.5 id 生成策略
- **进程内**：`format!("in_process_{phase}_{name}")`
  - `phase` = `format!("{:?}", meta.phase)`，形如 `PreUpload` / `Upload` / `PostUpload`
  - `name` = `meta.name`
- **dylib**：从 `<resource_dir>/plugin.id` 文件读取内容作为插件 id；**文件缺失 → `Err`**（外部插件必须携带 id 文件）。
- 进程内 id 不再含 `author`，但 `meta.author` 仍作展示元信息保留；dylib 唯一标识是文件 id，`meta.name` 仍作人类可读名，两者并存。

### 5.6 运行期流转（不变）
- `PluginRegistryInfo.registry_config: Option<Value>` 仍是裸 JSON、由代码注入。
- `execute_pipeline` 注入 `config_info: Arc<Option<Value>>` 现状不变。
- schema 纯透传，不与 `registry_config` 发生任何校验 / 合并。

## 6. 文件迁移、构建脚本、测试与范围

### 6.1 旧文件废弃
- 删除 `file_uploader_plugins/{pre_upload,upload,post_upload}_plugins.json`（聚合文件）。
- 删除 `uploader_example_plugin/config.json`（扁平形态）。

### 6.2 新增 / 改写
- `file_uploader_plugins/resources/pre/file_type_filter/config.json` 按新 schema 改写；`meta.json` 保留并核对字段。
- `uploader_example_plugin/`：新建 `meta.json`、按新 schema 改写 `config.json`、预置占位 `plugin.id`（CLI 落地前手写测试值，CLI 接管后覆盖）。

### 6.3 目标态目录结构
```
file_uploader_plugins/resources/pre/file_type_filter/{meta.json, config.json}
uploader_example_plugin/{meta.json, config.json, plugin.id}
target/debug/
  resources/pre/file_type_filter/{meta,config}.json     ← 进程内
  libuploader_example_plugin.dylib
  meta.json  config.json  plugin.id                     ← dylib 同目录
```

### 6.4 `build.rs` 改造
- `file_uploader_plugins/build.rs`：从「复制 3 个聚合 json」→「递归复制 `resources/` 整树到 `target/resources/`」（手写 `fs::read_dir` 递归，不引依赖）。
- `uploader_example_plugin/build.rs`：从「复制 `config.json`」→「复制 `meta.json` + `config.json` + `plugin.id` 三件到 dylib 同目录」。
- 两处补 `cargo:rerun-if-changed=<新路径>`。

### 6.5 入口调用更新
- `main.rs`：`new_in_process("./resources/pre/file_type_filter", Box::new(FileTypeFilter))`；`new_from_dylib_path("./libuploader_example_plugin.dylib")` 签名不变。
- `registry.rs`：`LazySlotSource::InProcess{ config_path, .. }` → `InProcess{ resource_dir, .. }`；`UploadPluginInfo{ default_config, .. }` → `UploadPluginInfo{ config, .. }`（含其测试夹具）。

### 6.6 测试覆盖
- 反序列化：`PluginConfigInfo` 的 `select` / `text` 两种 `form`（含 `#[serde(default)]` 省略字段场景）。
- `PluginResource::load`：成功 / `meta.json` 缺失报错 / `config.json` 解析失败报错（不再吞）/ `config.json` 缺失→空容器。
- id 生成：进程内 `in_process_{phase}_{name}` 公式；dylib 读取 `plugin.id` / 缺失报错。
- 回归：`registry.rs` 现有排序、pipeline 回调、config 注入测试（夹具适配新字段后应全绿）。

## 7. 改动文件清单（实现阶段参照）

| 文件 | 改动 |
|---|---|
| `file_uploader_core/src/pipeline/plugin.rs` | 新增 `PluginConfigInfo` / `PluginConfigItem` / `PluginFormSpec` / `PluginValueOption` / `PluginResource`；退役旧 `PluginConfig`；改造 `UploadPluginInfo`（字段 + 两入口 + id 生成）；修正 typo |
| `file_uploader_core/src/pipeline/registry.rs` | 适配 `LazySlotSource` 字段更名、`UploadPluginInfo.config` 字段（含测试夹具） |
| `file_uploader_core/src/main.rs` | 入口调用路径更新 |
| `file_uploader_plugins/resources/pre/file_type_filter/config.json` | 按新 schema 改写 |
| `file_uploader_plugins/{pre_upload,upload,post_upload}_plugins.json` | 删除 |
| `file_uploader_plugins/build.rs` | 递归复制 `resources/` |
| `uploader_example_plugin/{meta.json,config.json,plugin.id}` | 新建 / 改写 / 占位 |
| `uploader_example_plugin/config.json`（旧扁平） | 删除 |
| `uploader_example_plugin/build.rs` | 复制三件到 dylib 同目录 |
| `AGENTS.md` | 同步：项目结构、`PluginConfig` 类型族、配置文件格式节、`UploadPluginInfo`/id 策略、`build.rs` 描述（详见 §8.1） |
| `README.md` | 同步：插件配置特性、项目结构（`resources/`）、插件开发指南配置示例、Pipeline 使用示例入口路径（详见 §8.2） |

## 8. 文档同步要点

> 实现完成后同步修订，确保两份文档与重构后行为一致。

### 8.1 `AGENTS.md`
- **项目结构**：移除不存在的 `plugin.json` 描述；`file_uploader_plugins/` 增加 `resources/<phase>/<name>/{meta,config}.json`；`plugin.rs` 描述补入 `PluginConfigInfo / PluginConfigItem / PluginFormSpec / PluginResource`。
- **「插件配置（`PluginConfig`）」段**：更新为 `PluginConfigItem`（`key/title/description/config_type/default_value/form`）+ `PluginConfigInfo`（`access/params`）+ `PluginFormSpec`（`Text`/`Select`，表单驱动）。
- **「配置文件格式」段**：由「两种 JSON 格式（进程内嵌套 / dylib 扁平）」改为统一的目录化格式 —— 每插件一目录含 `meta.json` + `config.json`（`{ access, params:[{...form:{type,...}}] }`）；dylib 产物同目录额外含 `plugin.id`。给出新 schema 示例。
- **「插件信息（`UploadPluginInfo`）」段**：`new_in_process(resource_dir, plugin)` / `new_from_dylib_path` 读同目录 `meta.json`+`config.json`（dylib 另读 `plugin.id`）；id 生成策略 —— 进程内 `in_process_{phase}_{name}`、dylib 读取 `plugin.id`。
- **`build.rs` 段**：进程内递归复制 `resources/` 整树到 `target/resources/`；dylib 复制 `meta.json`+`config.json`+`plugin.id` 三件到产物同目录。
- **注意事项**：移除 `unknow` typo 相关注述（已修正为统一 `unknown`/公式生成）。

### 8.2 `README.md`
- **特性「插件配置」**（第 12 行）：由「每个 JSON 配置文件定义插件元数据与可配置项」改为目录化 `meta.json`+`config.json` + 表单驱动 schema（`form` / `text` / `select`）。
- **项目结构**：`file_uploader_plugins/` 增加 `resources/` 目录；`plugin.rs` 描述补入新类型族。
- **「开发进程内插件」配置示例**（第 147–168 行）：旧嵌套聚合格式 → 目录化 `meta.json` + `config.json`（含 `form:{type,...}`）示例。
- **「开发动态库插件」段**（第 170–208 行）：补「`.dylib` 同目录需含 `meta.json`+`config.json`+`plugin.id`」说明。
- **「使用 Pipeline」示例**（第 219 行）：`new_in_process("config.json", ...)` → `new_in_process("./resources/pre/my-plugin", ...)`。

## 9. TODO（范围外，独立后续）

- **插件构建 CLI**：新建插件项目时，基于「项目名 + 时间」hash 生成唯一 ID，写入项目根目录的 `plugin.id` 文件；构建时随 `.dylib` 产物分发到同目录。`plugin.id` 的文件格式（纯文本 vs 结构化）与 hash 算法在 CLI 设计阶段定稿，届时回看本 spec 第 5.5 节 dylib id 读取逻辑是否需同步调整。
