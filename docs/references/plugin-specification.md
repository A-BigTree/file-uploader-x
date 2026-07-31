# 插件规范

file-uploader-x 插件体系的完整规范。涵盖插件两种形态、资源目录约定、`config.json` Schema、
表单控件与约束、参数校验、Pipeline 注册与执行、README 编写规范、构建期资源复制。

> 面向插件开发者与框架维护者。插件**使用者**请看各插件资源目录下的 `README.md`。

## 目录

1. [插件体系](#1-插件体系)
2. [资源目录与 meta.json](#2-资源目录与-metajson)
3. [config.json Schema](#3-configjson-schema)
4. [表单控件与约束](#4-表单控件与约束)
5. [参数校验](#5-参数校验)
6. [Pipeline 注册与执行](#6-pipeline-注册与执行)
7. [README 规范](#7-readme-规范)
8. [构建与资源复制](#8-构建与资源复制)

---

## 1. 插件体系

### 1.1 两种插件形态

| 形态 | trait | 加载方式 | 封装 |
|---|---|---|---|
| 进程内插件 | `UploadPlugin` | 编译期链接 | `PluginSlot::InProcess` |
| 动态库插件 | `UploadDylibPlugin`（stabby ABI） | `libloading` 运行期加载 cdylib | `PluginSlot::Dylib` |

两者定义在 `file_uploader_sdk::models::interface`。

**`UploadPlugin`（进程内）**

```rust
pub trait UploadPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;                                    // 必须
    fn phase(&self) -> UploadPhase;                                    // 必须
    fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx;        // 必须
    fn on_load(&self) {}
    fn on_unload(&self) {}
    fn validate_params(&self, _ctx: &UploadInputCtx) -> Result<(), String> { Ok(()) }
}
```

**`UploadDylibPlugin`（动态库）**

```rust
#[stabby::stabby]
pub trait UploadDylibPlugin: Send + Sync {
    extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS;   // 必须
    extern "C" fn on_load(&self) {}
    extern "C" fn on_unload(&self) {}
    extern "C" fn set_logger(&self, _callback: PluginLogCallback) {}
    extern "C" fn validate_params(&self, _ctx: &UploadInputCtxS)
        -> stabby::option::Option<SString> { stabby::option::Option::None() }
}
```

动态库插件还需导出 `get_dylib_plugin`（类型 `FnGetDylibPlugin`），`Cargo.toml` 指定 `crate-type = ["cdylib"]`。

### 1.2 插件插槽与懒加载

- **`PluginSlot`**：统一封装两种来源，对外提供一致的 `execute` / `on_load` / `on_unload` / `validate_params`。
  实现 `Drop` 时自动调用 `on_unload`（手动 drop 时注意副作用）
- **`LazyPluginSlot`**：基于 `OnceLock` 延迟初始化，插件仅在首次 `execute` / `on_load` / `validate_params`
  时才真正加载。`UploadPluginRegistryTable::preload_all` 可主动预加载全部插件

**懒加载语义是一条硬约束**：`UploadPluginRegistryTable::new` 内的声明式校验绝不允许触发插件加载。

### 1.3 上传阶段

```
Input → PreUpload → Upload → PostUpload → Output
```

同阶段内按 `priority` 排序（值越小越先执行）。

### 1.4 核心数据流

```
UploadInputCtx → 插件处理 → UploadOutputCtx
```

插件输出经 `output_to_input` 转为下一插件输入，`extra_info` 累积传递。
插件返回 `Failed` 立即中断整条 pipeline。

**`UploadInputCtx` 关键字段**

| 字段 | 类型 | 说明 |
|---|---|---|
| `file` | `Option<Arc<UploadFileData>>` | 待处理文件 |
| `config_info` | `Arc<Option<Value>>` | 运行态配置（由 `registry_config` 注入） |
| `extra_info` | `Option<HashMap<String,String>>` | 插件间累积传递的附加信息 |
| `work_dir` | `Option<String>` | 本次流程的唯一工作目录（流程级常量，宿主预创建并透传） |

**`UploadOutputCtx` 构造**：优先用关联函数，不要手写字面量。

| 函数 | 语义 |
|---|---|
| `success(msg)` | 成功，无文件产出 |
| `success_file(msg, file)` | 成功并产出文件 |
| `failed(msg)` | 校验/处理失败，**中断 pipeline** |
| `interrupt(msg)` | 主动中断 |

**Upload 阶段产物约定**：上传类插件成功后应通过 `UploadFileData` 原生表达远程产物——
把 `data_type` 改写为 `NetworkPath`、`input_path` 设为可公开访问的 URL；本地路径、object key、
provider 等辅助信息只放进 `extra_info`。下游 Output 插件统一消费这个 `NetworkPath` 文件，
不绑定某个上传插件的私有 key，从而保证上传器与输出器可自由组合。

### 1.5 Stabby ABI 兼容层

Rust 原生类型经 `models/ctx_stabby.rs` 的 `*S` 结构体映射到 stabby 类型：

| 原生 | stabby |
|---|---|
| `String` | `SString` |
| `Option<T>` | `SOption<T>` |
| `Arc<T>` | `SArc<TS>` |
| `Vec<u8>` | `SVec<u8>` |
| `serde_json::Value`（`config_info`） | `SOption<SString>`（**JSON 字符串**） |
| `HashMap<String,String>`（`extra_info`） | `SOption<SString>`（**JSON 字符串**） |

`utils/ctx_util.rs` 提供双向转换：`convert_input_ctx_s` / `convert_input_ctx` /
`convert_file_data(_s)` / `convert_output_ctx`。

所有跨 ABI 枚举均标注 `#[stabby::stabby]` + `#[repr(u8)]`。

> **ABI 破坏性变更**：给 `UploadDylibPlugin` 新增方法、或给 stabby 结构体增删字段，
> 都会改变 vtable / 内存布局，旧 `.dylib` 产物与新宿主**不兼容**（即使新方法有默认实现，
> 默认实现是编译期为具体类型写入 vtable 的，老产物缺少该槽位）。
> 新增方法务必**追加在 trait 末尾**，并 `cargo build --workspace` 全量重编。

### 1.6 插件日志

| 形态 | 方式 |
|---|---|
| 进程内 | 直接用 `tracing` 宏（`info!` / `error!` 等） |
| 动态库 | 用 SDK 的 `plugin_*!` 宏（`plugin_info!` / `plugin_error!` 等） |

动态库插件**必须**实现 `set_logger` 并调用 `set_logger_callback(callback)`，否则日志被静默忽略。
宿主侧回调实现在 `file_uploader_core/src/pipeline/plugin.rs` 的 `plugin_log_callback`。

> **安全**：不要整体序列化 `ctx` 打日志 —— 会泄漏 `secret: true` 字段的值。只打印必要的非敏感字段。

---

## 2. 资源目录与 meta.json

统一的目录化格式，每个插件一个资源目录。

**进程内插件**：`file_uploader_plugins/resources/<phase>/<plugin_name>/`

**动态库插件**：`.dylib` 产物同目录

| 文件 | 必需 | 内容 |
|---|---|---|
| `meta.json` | 是 | `PluginMeta` |
| `config.json` | 否 | `PluginConfigInfo`；缺失 → 空容器 |
| `README.md` | 否 | 使用说明书；**只记录路径不加载内容** |
| `plugin.id` | dylib 必需 | 插件唯一 ID，由插件构建 CLI 生成 |

`<phase>` 段约定：`input` → Input、`pre` → PreUpload、`upload` → Upload、`post` → PostUpload、`output` → Output。
Rust 侧模块目录对应 `input` / `pre_upload` / `upload` / `post_upload` / `output`。

### meta.json

```json
{
  "name": "upload_file_validator",
  "title": "上传文件校验器",
  "description": "按文件类型、文件名、大小三维校验单个上传文件",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "PreUpload"
}
```

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `name` | string | 是 | 插件唯一名，与资源目录名、`UploadPlugin::name()` 保持一致 |
| `title` | string | 是 | 展示名（中文） |
| `description` | string | 是 | 一句话说明 |
| `version` | string | 是 | 语义化版本 |
| `author` | string / null | 是 | 作者，可为 `null` |
| `phase` | enum | 是 | `Input` / `PreUpload` / `Upload` / `PostUpload` / `Output`（大驼峰） |

### 插件资源加载（`PluginResource`）

公共加载器 `PluginResource::load(dir)`：

1. `meta.json` —— 必读，缺失 → `UploadError::PluginLoadError`
2. `config.json` —— 选读，缺失 → `PluginConfigInfo::default()`（空容器）
3. `README.md` —— 选读，存在则记录路径到 `readme_path`，缺失 → `None`（不报错不告警）

### 插件 ID 生成

| 形态 | 规则 | 示例 |
|---|---|---|
| 进程内 | `format!("in_process_{:?}_{}", phase, name)` | `in_process_PreUpload_upload_file_validator` |
| 动态库 | 读取产物同目录 `plugin.id` 文件内容（trim） | `dylib_uploader_test_example_plugin_20260616_0001` |

注意进程内 ID 用 `{:?}` 格式化 phase，得到的是 `PreUpload` 这类大驼峰形态。

### `UploadPluginInfo`

封装插件 ID、`meta`、`config`（`Arc<PluginConfigInfo>`）、加载路径 `path`、`readme_path`、`LazyPluginSlot`。

两个构造入口共用 `PluginResource::load`：

- `new_in_process(resource_dir, Box<dyn UploadPlugin>)`
- `new_from_dylib_path(dylib_path)` —— 资源目录取 `dylib_path.parent()`

---

## 3. config.json Schema

Schema 类型定义在 `file_uploader_sdk::models::config_schema`
（下沉到 SDK 便于 dylib 插件复用），`file_uploader_core::pipeline::plugin` 通过 `pub use` 重导出。

### 3.1 三段结构

```json
{
  "access":  { },
  "common":  [ ],
  "groups":  [ ]
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `access` | `PluginAccessConfig` | 权限声明，纯透传 |
| `common` | `PluginConfigItem[]` | 跨分组公共参数，**始终生效** |
| `groups` | `PluginConfigGroup[]` | **互斥分组**（工作模式），运行态只激活一个；可为空 |

三个字段均可省略（走 serde default）。

### 3.2 access —— 权限声明

```json
{
  "access": { "fs_read": ["/tmp/uploads"], "fs_write": false, "network": true }
}
```

三个权限点 + `extra` 预留扩展位。每项为 `AccessSpec`：

| 写法 | 语义 |
|---|---|
| `true` | 全开 |
| `false` | 全关（**默认**） |
| `["/tmp/a", "/data/b"]` | 白名单：fs 为路径，network 为 host |

**当前纯透传，不做执行逻辑**。未来会结合 `work_dir` 沙箱做权限收敛（`fs_util::check` 已留扩展点）。

### 3.3 groups —— 互斥分组

`group` 是**互斥的模式类型**语义（如 `oss` / `s3` / `local`），运行态**只激活一个**。

```json
{
  "groups": [
    {
      "group": "oss",
      "title": "阿里云 OSS",
      "description": "上传到对象存储",
      "params": [ /* PluginConfigItem[] */ ]
    },
    { "group": "local", "title": "本地存储", "params": [ ] }
  ]
}
```

| 字段 | 必需 | 说明 |
|---|---|---|
| `group` | 是 | 分组标识，运行态 `registry_config` 的 `group` 值 |
| `title` | 是 | 展示名 |
| `description` | 否 | 分组说明 |
| `params` | 否 | 该分组独有参数 |

**约定**

| 情况 | 运行态配置 |
|---|---|
| `groups` 为空 / 省略（单形态插件） | **不应**出现 `group` 字段，否则报 `GroupNotAllowed` |
| `groups` 非空（多形态插件） | **必须**带 `group`，且只能取声明的分组标识之一 |

**生效参数集** = `common` + 激活分组的 `params`。未激活分组的参数完全不参与校验。

**辅助方法**（`PluginConfigInfo`）

| 方法 | 用途 |
|---|---|
| `has_groups()` | 是否为分组型插件 |
| `find_group(g)` | 按标识查分组 |
| `group_keys()` | 全部合法分组标识（用于 UI 下拉与报错提示） |
| `effective_items(Some(g))` | 生效参数集 |

### 3.4 params —— 配置项

```json
{
  "key": "max_size",
  "title": "最大体积",
  "description": "支持 10mb 这类带单位的写法；填 0 表示不限",
  "config_type": "Custom",
  "default_value": "0",
  "required": false,
  "form": { "type": "text", "max_len": 16 }
}
```

| 字段 | 必需 | 说明 |
|---|---|---|
| `key` | 是 | 参数键，与运行态配置的 key 对齐 |
| `title` | 是 | 展示名（中文），README 与错误信息中使用 |
| `description` | 否 | 参数说明 |
| `config_type` | 是 | `Default` / `Custom` |
| `default_value` | 否 | 默认值，**仅供 UI 预填**（见 3.5） |
| `required` | 否 | 通用约束，默认 `false` |
| `form` | 是 | 表单控件描述 + 控件级约束（见第 4 节） |

### 3.5 default_value 不做合并

`default_value` **只供 UI 预填**，框架**不会**把它合并进运行态配置。

因此插件读配置时**必须自行兜底**：

```rust
let strict = config_util::get_bool(&ctx.config_info, "strict_mode").unwrap_or(false);
```

`default_value` 与代码里的兜底值应保持一致，避免文档与实际行为不符。

### 3.6 完整示例

**单形态插件**（`groups` 为空）

```json
{
  "access": { "fs_read": false, "fs_write": false, "network": false },
  "common": [
    {
      "key": "pass_type", "title": "允许类型",
      "description": "为空表示全部允许，支持 glob 如 image/*",
      "config_type": "Custom", "default_value": [], "required": false,
      "form": {
        "type": "select", "multiple": true, "allow_custom": true, "max_items": 50,
        "options": [
          { "label": "图片(image/*)", "value": "image/*" },
          { "label": "PDF", "value": "application/pdf" }
        ]
      }
    },
    {
      "key": "max_size", "title": "最大体积",
      "description": "支持 1kb/1mb/1g；为 0 或空表示不限",
      "config_type": "Custom", "default_value": "0", "required": false,
      "form": {
        "type": "text", "max_len": 16,
        "pattern": "^\\s*\\d+\\s*(?i:b|byte|bytes|k|kb|m|mb|g|gb|t|tb)?\\s*$",
        "placeholder": "如 10mb"
      }
    },
    {
      "key": "strict_mode", "title": "严格模式",
      "description": "开启后文件类型无法识别时直接拒绝",
      "config_type": "Default", "default_value": false, "required": false,
      "form": { "type": "switch" }
    }
  ],
  "groups": []
}
```

**多形态插件**（`common` + `groups`）

```json
{
  "access": { "fs_read": true, "fs_write": false, "network": true },
  "common": [
    {
      "key": "retry_times", "title": "重试次数", "description": "失败后重试次数",
      "config_type": "Default", "default_value": 3, "required": false,
      "form": { "type": "number", "min": 0, "max": 10, "step": 1, "integer": true }
    }
  ],
  "groups": [
    {
      "group": "oss", "title": "阿里云 OSS", "description": "上传到阿里云对象存储",
      "params": [
        {
          "key": "endpoint", "title": "Endpoint", "description": "OSS 服务地址",
          "config_type": "Default", "default_value": "", "required": true,
          "form": {
            "type": "text", "min_len": 8, "max_len": 256,
            "pattern": "^https?://.+",
            "placeholder": "https://oss-cn-hangzhou.aliyuncs.com"
          }
        },
        {
          "key": "access_secret", "title": "AccessKey Secret",
          "description": "访问凭证密钥（密码框，前端不回显）",
          "config_type": "Default", "default_value": "", "required": true,
          "form": { "type": "text", "secret": true, "min_len": 1, "max_len": 256 }
        },
        {
          "key": "use_https", "title": "使用 HTTPS", "description": "是否加密传输",
          "config_type": "Default", "default_value": true, "required": false,
          "form": { "type": "switch" }
        }
      ]
    },
    {
      "group": "local", "title": "本地存储", "description": "上传到本机目录",
      "params": [
        {
          "key": "base_dir", "title": "存储根目录", "description": "文件落地的绝对路径",
          "config_type": "Default", "default_value": "/tmp/uploads", "required": true,
          "form": { "type": "text", "min_len": 1, "max_len": 512, "pattern": "^/.*" }
        },
        {
          "key": "naming", "title": "命名策略", "description": "落地文件的命名方式",
          "config_type": "Default", "default_value": "uuid", "required": true,
          "form": {
            "type": "select", "multiple": false, "allow_custom": false,
            "options": [
              { "label": "原始文件名", "value": "origin" },
              { "label": "UUID", "value": "uuid" },
              { "label": "内容哈希", "value": "hash" }
            ]
          }
        }
      ]
    }
  ]
}
```

---

## 4. 表单控件与约束

`form` 为 serde **internally tagged enum**（tag = `type`，值小写），四个变体。
控件级约束**内嵌在同一个 form 对象里**，所有约束字段均可省略。

### 4.1 控件总表

| `type` | 期望值类型 | 约束字段 |
|---|---|---|
| `text` | string | `secret` / `min_len` / `max_len` / `pattern` / `placeholder` |
| `switch` | bool | 无 |
| `select` | 标量（单选）或数组（多选） | `options` / `multiple` / `allow_custom` / `min_items` / `max_items` |
| `number` | number | `min` / `max` / `step` / `integer` |

顶层通用约束 `required` 位于 `PluginConfigItem`，适用所有控件。

### 4.2 text

```json
{ "type": "text", "secret": true, "min_len": 1, "max_len": 256,
  "pattern": "^https?://.+", "placeholder": "https://..." }
```

| 字段 | 默认 | 说明 |
|---|---|---|
| `secret` | `false` | `true` 为密码框，前端不回显 |
| `min_len` / `max_len` | 无 | 长度按**字符数**计（`chars().count()`），非字节数 |
| `pattern` | 无 | Rust `regex` 语法，按 `is_match` 判定（未锚定即子串匹配）。JSON 中反斜杠要写 `\\` |
| `placeholder` | 无 | 纯 UI 提示，不参与校验 |

### 4.3 switch

```json
{ "type": "switch" }
```

无约束字段。值必须是 JSON `true` / `false`，**不接受** `"true"` 字符串。

### 4.4 select

```json
{ "type": "select", "multiple": true, "allow_custom": false,
  "min_items": 1, "max_items": 5,
  "options": [ { "label": "PNG", "value": "image/png" } ] }
```

| 字段 | 默认 | 说明 |
|---|---|---|
| `options` | `[]` | 候选项 `{label, value}`，`value` 为任意 JSON 值 |
| `multiple` | `false` | `true` 时值须为数组，元素须为标量 |
| `allow_custom` | `false` | `false` 时值必须在 `options` 内；`true` 时任意值放行 |
| `min_items` / `max_items` | 无 | 仅 `multiple: true` 时的选中数量约束 |

### 4.5 number

```json
{ "type": "number", "min": 0, "max": 10, "step": 1, "integer": true }
```

| 字段 | 默认 | 说明 |
|---|---|---|
| `min` / `max` | 无 | 闭区间，浮点比较带 `1e-9` 容差 |
| `step` | 无 | 校验 `(v - min.unwrap_or(0))` 是否为 `step` 的整倍数（容差 `1e-6`） |
| `integer` | `false` | `true` 时小数部分须为 0 |

### 4.6 选型要点

| 需求 | 选择 | 不要这样做 |
|---|---|---|
| 布尔开关 | `switch` | ✗ 用 `text` 存 `"true"` 字符串 |
| 整数 / 浮点 | `number` | ✗ 用 `text` 存数字字符串 |
| 带单位的量（`10mb`） | `text` + `pattern` | —— 这是 `text` 的合理用法 |
| 密钥 / token | `text` + `"secret": true` | ✗ 明文 `text` |
| 固定候选项 | `select` + `allow_custom: false` | ✗ 用 `text` 让用户自己拼 |
| 开放式多选（MIME 列表） | `select` + `multiple` + `allow_custom` | —— |

---

## 5. 参数校验

**双层校验**：框架声明式约束**先跑**，全部通过后才调插件 `validate_params`。

### 5.1 声明式校验

实现在 `file_uploader_sdk::utils::validate_util`。

**对外入口**

| 函数 | 用途 |
|---|---|
| `validate_plugin_config(config, values)` | 主入口 |
| `validate_plugin_config_opt(config, &Option<Value>)` | 直接吃 `registry_config`，`None` 等价空对象 |
| `validate_plugin_config_with(config, values, &ValidateOptions)` | 带选项（目前仅 `strict_unknown_keys`） |
| `validate_item(item, group, value)` | 单项校验，供前端逐字段实时校验复用 |
| `is_empty_value(v)` | 判空语义 |
| `errors_to_string(&errs)` | 多条错误折叠为单行 |

**校验流程**（错误**全部累积不短路**）

1. `values` 非 JSON object → `NotAnObject`，立即返回
2. 分组校验：
   - `has_groups()` 且 `group` 缺失/非字符串 → `GroupMissing`
   - `has_groups()` 且 `group` 不在 `group_keys()` → `GroupUnknown`
   - `!has_groups()` 却传了 `group` → `GroupNotAllowed`
3. 对 `effective_items(group)` 逐项校验：
   `required` 判空 → **空值直接跳过后续约束** → 类型匹配 → 控件约束
4. `strict_unknown_keys` 开启时，扫描 `values` 中不属于生效 key ∪ `{group}` 的键 → `UnknownKey`

**判空语义**：缺失 / `null` / `""` / `[]` 均视为未填。
`false` 与 `0` **不算**空值。

**空值短路**：值为空且 `required: false` 时，不再跑长度/正则/范围等约束 —— 避免"没填却报格式错"。

### 5.2 ValidationError

```rust
pub struct ValidationError {
    pub key: Option<String>,     // 参数键；分组级/顶层错误为 None
    pub group: Option<String>,   // 所属分组；common 参数为 None
    pub title: Option<String>,   // 参数展示名，便于前端直接展示
    pub reason: ValidationReason,
}
```

实现了 `Display`，格式为 `[group.key] 原因` / `[key] 原因` / `原因`。

**ValidationReason 全表**

| 变体 | 触发条件 |
|---|---|
| `NotAnObject` | 运行态配置不是 JSON object |
| `GroupMissing { expected }` | 分组型插件未提供 `group` |
| `GroupUnknown { found, expected }` | `group` 值不在声明的分组内 |
| `GroupNotAllowed` | 非分组型插件却传了 `group` |
| `Required` | 必填项未填（含 `null` / `""` / `[]`） |
| `TypeMismatch { expected, found }` | 值类型与控件不匹配 |
| `TooShort { min, actual }` | 文本长度低于 `min_len` |
| `TooLong { max, actual }` | 文本长度超过 `max_len` |
| `PatternMismatch { pattern }` | 文本不满足 `pattern` |
| `InvalidPattern { pattern, error }` | **schema 自身**的 `pattern` 正则非法 |
| `NotInOptions { allowed }` | `select` 非 `allow_custom` 且值不在 `options` 内 |
| `TooFewItems { min, actual }` | 多选数量低于 `min_items` |
| `TooManyItems { max, actual }` | 多选数量超过 `max_items` |
| `OutOfRange { min, max }` | 数值越界 |
| `NotInteger` | `integer: true` 却传了小数 |
| `StepMismatch { step }` | 数值不是 `step` 的整倍数 |
| `UnknownKey` | 严格模式下的未声明字段 |
| `PluginRejected { message }` | 插件 `validate_params` 拒绝 |

### 5.3 插件级 validate_params

只做声明式约束**表达不了**的校验：

| 适合 | 例子 |
|---|---|
| 跨字段一致性 | A 开启时 B 必填 |
| 需调用库才能判定的格式 | glob 模式合法性、`parse_size` 可解析性 |
| 业务白名单 | `group` 只支持已实现的几种 |

**不要**在这里重复做 required / 长度 / 正则 / 范围校验 —— 那些写进 `config.json` 即可。

**进程内插件**

```rust
fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
    if let Some(raw) = config_util::get_str(&ctx.config_info, "max_size") {
        let t = raw.trim();
        if !t.is_empty() && config_util::parse_size(t).is_none() {
            return Err(format!("max_size 无法解析: '{raw}'"));
        }
    }
    for key in ["pass_type", "reject_type", "pass_name"] {
        for p in config_util::get_list(&ctx.config_info, key) {
            Pattern::new(&p).map_err(|e| format!("{key} 含非法 glob '{p}': {e}"))?;
        }
    }
    Ok(())
}
```

**动态库插件**（`Some(msg)` = 失败，`None` = 通过）

```rust
extern "C" fn validate_params(&self, ctx: &UploadInputCtxS)
    -> stabby::option::Option<SString> {
    let ctx = convert_input_ctx(ctx);
    match config_util::get_group(&ctx.config_info).as_deref() {
        Some("oss" | "local") => stabby::option::Option::None(),
        Some(other) => stabby::option::Option::Some(format!("未知分组 '{other}'").into()),
        None => stabby::option::Option::Some("缺少分组标识 group".into()),
    }
}
```

**统一转发链**

```
PluginRegistryInfo::validate_params
  → UploadPluginInfo::validate_params
    → LazyPluginSlot::validate_params   （触发懒加载，错误映射为 UploadError::PluginParamInvalid）
      → PluginSlot::validate_params     （dylib 侧 convert_input_ctx_s + SOption<SString> → Result）
```

### 5.4 校验时机

| 时机 | 声明式 | 插件级 | 是否加载插件 | 失败行为 |
|---|---|---|---|---|
| `UploadPluginRegistryTable::new` | ✓ | ✗ | **否** | 仅缓存 + `warn!` |
| `try_new` | ✓ | ✗ | **否** | 返回 `Err(Vec<(plugin_id, ValidationError)>)` |
| `preload_all` | 复用缓存 | ✓ | 是 | 返回 `Err(Vec<UploadError>)` |
| `validate_all()` | ✓ | ✓ | 是 | 返回 `Err(Vec<UploadError>)` |
| `validate_plugin_config(...)` | ✓ | ✗ | 否 | 返回 `Err(Vec<ValidationError>)` |

**设计要点**

- `new` 保持原签名不返回 `Result`，且**绝不加载插件** —— 保住懒加载语义
- 结构化错误明细通过 `table.declarative_errors()` 获取；
  `preload_all` / `validate_all` 会把它折叠为 `UploadError::PluginConfigInvalid(String)`
- 声明式已失败的插件不再跑插件级校验，避免错误噪声
- `validate_plugin_config` 是纯函数，供宿主/前端在**保存配置前**预校验

### 5.5 错误类型

`UploadError` 相关变体：

| 变体 | 来源 |
|---|---|
| `PluginConfigInvalid(String)` | 声明式校验失败（折叠后的明细） |
| `PluginParamInvalid(String)` | 插件 `validate_params` 返回的错误 |
| `PluginLoadError(String)` | 资源缺失、dylib 加载失败、符号找不到 |
| `WorkDirNotSet` | `work_dir` 为空/`None` 时调 `fs_util` IO |
| `WorkDirPathEscape { work_dir, path }` | 路径越出 work_dir 沙箱 |

---

## 6. Pipeline 注册与执行

### 6.1 运行态配置形状

`registry_config` 为**扁平一层 JSON**，保留字段 `group` 标识激活的分组，其余为参数 KV：

```json
{
  "group": "oss",
  "retry_times": 3,
  "endpoint": "https://oss-cn-hangzhou.aliyuncs.com",
  "access_secret": "SK-xxx"
}
```

单形态插件不含 `group`。经 `execute_pipeline` 注入到 `ctx.config_info`。

### 6.2 配置读取 helper

`file_uploader_sdk::utils::config_util` —— **优先使用，不要手写 `Value` 解析**。

| helper | 返回 | 对应控件 |
|---|---|---|
| `get_group(&cfg)` | `Option<String>` | 读激活分组标识 |
| `get_str(&cfg, k)` | `Option<String>` | `text` |
| `get_bool(&cfg, k)` | `Option<bool>` | `switch` |
| `get_i64(&cfg, k)` | `Option<i64>` | `number` + `integer` |
| `get_f64(&cfg, k)` | `Option<f64>` | `number` 浮点 |
| `get_list(&cfg, k)` | `Vec<String>` | `select` + `multiple`（非数组/缺失 → 空） |
| `get_size(&cfg, k)` | `Option<u64>` | 带单位体积（字符串走 `parse_size`，数字走 `as_u64`） |
| `parse_size(s)` | `Option<u64>` | 单独解析 `10mb`（1024 进制，单位大小写不敏感） |

多形态插件按分组分派：

```rust
match config_util::get_group(&ctx.config_info).as_deref() {
    Some("oss")   => self.upload_oss(ctx),
    Some("local") => self.save_local(ctx),
    other => UploadOutputCtx::failed(format!("不支持的分组: {other:?}")),
}
```

### 6.3 PluginRegistryInfo

```rust
pub struct PluginRegistryInfo {
    pub plugin_instance: Arc<UploadPluginInfo>,
    pub priority: i32,                        // 值越小越先执行
    pub status: PluginRegistryStatus,          // Enable / Disable
    pub registry_config: Option<Value>,        // 运行态实际配置
}
```

实现 `Ord`：先按阶段排序，同阶段按 `priority` 排序。

校验方法：`validate_declarative()`（不加载插件）/ `validate_params()`（触发懒加载）。

> **已知缺陷**：`status`（Enable/Disable）在 `execute_pipeline` 中**未被检查**，
> 禁用插件依然会执行。同理 `OutputResultType::Interrupt` 未被特殊处理，等同 Success 继续往下。

### 6.4 UploadPluginRegistryTable

| 方法 | 说明 |
|---|---|
| `new(id, plugins)` | 排序 + 声明式校验（不加载插件），错误缓存并 warn |
| `try_new(id, plugins)` | 同上，但声明式失败即返回 `Err` |
| `declarative_errors()` | 构建期缓存的结构化错误明细 |
| `get_plugins_by_phase(phase)` | 取某阶段插件 |
| `preload_all()` | 预加载全部插件 + 插件级 `validate_params` |
| `validate_all()` | 声明式 + 插件级全量校验 |
| `execute_pipeline(input_ctx, callback)` | 按阶段执行插件链 |

### 6.5 注册示例

```rust
use file_uploader_core::pipeline::plugin::UploadPluginInfo;
use file_uploader_core::pipeline::registry::{
    PluginRegistryInfo, PluginRegistryStatus, UploadPluginRegistryTable,
};

let info = UploadPluginInfo::new_in_process(
    "./target/debug/resources/pre/upload_file_validator",
    Box::new(UploadFileValidator),
)?;

let reg = PluginRegistryInfo::new(
    std::sync::Arc::new(info),
    1,
    PluginRegistryStatus::Enable,
    Some(serde_json::json!({ "pass_type": ["image/*"], "max_size": "10mb" })),
);

// 严格构建：配置不合法直接失败
let table = UploadPluginRegistryTable::try_new("pipeline_id".into(), vec![reg])
    .map_err(|errs| format!("config invalid: {errs:?}"))?;

// 预加载 + 插件级校验
table.preload_all().map_err(|errs| format!("{errs:?}"))?;

let out = table.execute_pipeline(input_ctx, None);
```

### 6.6 execute_pipeline

按 `Input → PreUpload → Upload → PostUpload → Output` 顺序执行。

对每个插件：

1. 构造该插件的输入 —— `file` / `extra_info` / `work_dir` 沿用当前上下文，
   `config_info` 注入该插件的 `registry_config`
2. 执行，返回 `Failed` 立即中断整条 pipeline
3. 输出经 `output_to_input` 转为下一插件输入（`extra_info` 累积合并，
   `config_info` 清空 —— 每个插件的配置单独注入）

### 6.7 Pipeline 事件回调

`file_uploader_core/src/pipeline/callback.rs`。

```rust
pub trait PipelineCallback: Send + Sync {
    fn on_event(&self, event: &PipelineEvent, ctx: &UploadInputCtx,
                result: Option<&UploadOutputCtx>);
}
```

**`PipelineEventKind`**：`PhaseStart` / `PhaseEnd` / `PluginStart` / `PluginEnd`

**`PipelineEvent`** 字段：`timestamp_ms`（回调瞬间的本地毫秒时间戳）、`kind`、`phase`、
`plugin_id`、`plugin_meta`。阶段级事件的 `plugin_id` 与 `plugin_meta` 为 `None`。

### 6.8 work_dir 沙箱与文件 IO

插件读写中间文件**必须走 `file_uploader_sdk::utils::fs_util`**，**禁止直接用 `std::fs`**。

| API | 说明 |
|---|---|
| `write(work_dir, ext, reader)` | 写文件，**强制使用生成的唯一名** `{ts}_{hash}.{ext}` |
| `write_with_gen(.., gen_fn)` | 自定义命名 |
| `open_read(work_dir, filename)` | 流式读，返回 `BufReader<File>` |
| `read_to_end` / `read_to_string` | 便捷读取 |
| `import_file(work_dir, src, ext)` | 从外部路径导入沙箱 |
| `read_external_head(path, n)` | 读外部文件前 n 字节（用于类型嗅探） |
| `file_size(work_dir, filename)` | 沙箱内文件大小 |
| `resolve` | 路径词法规范化 + 前缀校验 |
| `create_work_dir(base, id)` | 宿主侧创建活动目录 |
| `exists` / `create_dir` | 其它 |

**强制力边界**：dylib 插件直接调 libc / `std::fs` 无法被拦截，沙箱仅覆盖「走 `fs_util`」的路径；
`BufReader<File>` 为 std 类型，不跨 dylib ABI 边界传递。

---

## 7. README 规范

每个插件资源目录可放 `README.md`，**可选**（缺失不报错不告警）。
框架**不加载内容到内存**，`PluginResource` / `UploadPluginInfo` 只记录 `readme_path`。

### 7.1 定位

**面向配置者的使用说明书，不是开发文档。**

读者是「要用这个插件的人」，关心的是「这插件能帮我做什么、我该怎么填参数、填完会发生什么」。

### 7.2 推荐章节

```
# <插件中文名>
一句话说清它解决什么问题

## 能做什么      —— 用大白话列举能力，不谈实现
## 什么时候用它  —— 典型场景表：场景 → 怎么设
## 怎么配        —— 按「目的」分小节，每节讲清填什么 + 效果是什么
## 参数一览      —— 表格：参数 | 作用 | 默认 | 怎么填
## 常见问题      —— Q&A，覆盖真实会踩的坑与排查顺序
```

### 7.3 写作要求

- 用参数的**中文标题**（如「允许类型」），不要用代码里的 `key`
- 讲「效果」不讲「机制」：写「png 通过、pdf 被拒」，不写「命中 glob 后进入 keep 判定」
- 多形态插件要说清各模式**互斥**，以及切换模式后原参数不再生效
- 「常见问题」按真实排查顺序编写，给出可执行的定位步骤
- **不要**写：输入输出契约、`ctx` 字段名、Rust 类型名、trait 方法、错误码表、变更记录、阶段枚举
- 权限、所属阶段已在 `meta.json` / `config.json` 声明，README 不必重复
- **安全**：含 `secret` 的参数（密钥、Token）绝不得出现在插件日志或错误消息中；
  打印排错信息时只输出非敏感字段（状态码、object key 等）

### 7.4 范例

- `file_uploader_plugins/resources/pre/upload_file_validator/README.md`
- `file_uploader_plugins/resources/input/default_input_handler/README.md`
- `file_uploader_plugins/resources/upload/common_uploader/README.md`（多形态：Cloudflare R2）
- `file_uploader_plugins/resources/output/common_output/README.md`
- `uploader_example_plugin/README.md`（多形态插件）

---

## 8. 构建与资源复制

### 8.1 进程内插件

`file_uploader_plugins/build.rs` 递归复制 `resources/` **整树**到 `target/<profile>/resources/`。

```
file_uploader_plugins/resources/pre/upload_file_validator/{meta,config}.json + README.md
  ↓
target/debug/resources/pre/upload_file_validator/{meta,config}.json + README.md
```

新增插件自动覆盖，无需改 `build.rs`。`cargo:rerun-if-changed=resources` 已覆盖整树。

> **易踩**：改的是源 `resources/`，运行时读的是 `target/.../resources/` 副本 —— 改完要重新 build。

### 8.2 动态库插件

`uploader_example_plugin/build.rs` 复制到 `target/<profile>/`，与 `.dylib` 产物同目录
（`new_from_dylib_path` 靠 `parent()` 定位资源）：

| 文件 | 缺失行为 |
|---|---|
| `meta.json` / `config.json` / `plugin.id` | **panic**（必需） |
| `README.md` | 跳过并打 info（可选） |

> **已知隐患**：多个 dylib 插件共享 `target/<profile>/` 根目录时，
> `meta.json` / `config.json` / `plugin.id` 会互相覆盖。

### 8.3 验证命令

```bash
# 全量编译（dylib ABI 变更后必须全量重编）
cargo build --workspace

# 测试
cargo test --workspace
cargo test -p file_uploader_sdk validate_util      # 校验器专项

# 运行时冒烟
cargo build -p uploader_example_plugin && cargo run -p file_uploader_core

# 产物核对
ls target/debug/resources/pre/upload_file_validator/   # meta.json config.json README.md
ls target/debug/README.md target/debug/config.json target/debug/plugin.id
```

---

## 附：相关文档

- 新增进程内插件的操作手册：`.agents/skills/designing-in-process-plugins/SKILL.md`
- 历史设计与实现计划：`docs/superpowers/specs/` 与 `docs/superpowers/plans/`
