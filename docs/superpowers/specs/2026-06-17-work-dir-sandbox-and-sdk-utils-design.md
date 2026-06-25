# inputCtx 活动目录字段 + SDK 共性工具抽取设计

## 背景

当前 `UploadInputCtx`（`file_uploader_sdk/src/models/ctx.rs:178`）仅含 `file_list / config_info / extra_info / related_process_info`，插件在执行过程中产生的中间文件没有统一的存放位置，散落各处、难以隔离与清理。

另一方面，现有内置插件 `file_type_filter`（`file_uploader_plugins/src/pre_upload/file_type_filter.rs`）中出现多处可复用的样板操作（从 `config_info` 读列表、构造输出结果），新插件会反复重写。

## 目标

1. **活动目录（work_dir）**：在 `UploadInputCtx` 新增 `work_dir` 字段，作为一次执行流程的唯一工作目录，集中存放该流程的全部中间文件；以「强制沙箱」理念约束插件文件读写范围。
2. **SDK 共性抽取**：将插件通用操作（work_dir 创建与沙箱 IO、配置读取、输出构造）下沉到 `file_uploader_sdk`，降低插件样板代码。

## 范围与非目标

- **范围**：inputCtx 字段与 stabby 层同步、双向转换、pipeline 透传；SDK 新增 `fs_util` / `config_util` 与输出构造关联函数；`file_type_filter` 改造为使用新工具。
- **非目标**（显式排除，留待后续）：
  - 不在本期实现 work_dir 与 `config.json` 的 `access`（fs_read/fs_write/network）白名单交集收敛，仅预留扩展点。
  - 不在 SDK 集成网络下载能力，后续在 `file_uploader_core` 中集成。
  - 不抽取 glob 匹配工具（YAGNI，待出现第二个消费者再做）。
  - 不为 dylib 插件拦截其直接调用 libc / `std::fs` 的路径（FFI 层无法实现），强制力仅覆盖「走 SDK 提供的 IO 函数」的路径。

## 关键决策（已确认）

| 维度 | 决策 |
|------|------|
| 约束强度 | 强制沙箱（硬约束），但对 dylib 仅约束走 SDK API 的路径 |
| 字段语义 | 一次执行流程的唯一工作目录，集中存放中间文件；流程级常量 |
| 字段结构 | 单个 `work_dir: Option<String>`（`None` = 未设置） |
| 字段来源 | 宿主预创建并透传；SDK 也提供 `create_work_dir` 创建方法 |
| 实现方案 | 方案 A：自由函数沙箱，工具统一置 `utils/` |
| 写文件语义 | 强制使用生成的唯一文件名（不接收调用方文件名/路径），预留自定义生成器 |
| 读写形态 | 统一流式传输（基于 `impl Read` / `BufReader<File>`），另保留 `read_to_end` / `read_to_string` 便捷封装 |
| access 预留 | 仅在 `fs_util::resolve` 内预留 `check()` 扩展点，本期不实现收敛 |

## 详细设计

### 1. 数据结构变更

**`UploadInputCtx`（`models/ctx.rs`）新增字段：**

```rust
pub struct UploadInputCtx {
    pub file_list: Vec<Arc<UploadFileData>>,
    pub config_info: Arc<Option<Value>>,
    pub extra_info: Option<HashMap<String, String>>,
    #[serde(skip)]
    pub related_process_info: Option<Weak<UploadProcessCtx>>,
    /// 活动目录：本次执行流程的唯一工作目录，集中存放中间文件。
    /// None 表示未设置（无沙箱约束信息）。流程级常量，由宿主预创建并透传。
    #[serde(default)]
    pub work_dir: Option<String>,
}
```

**`UploadInputCtxS`（`models/ctx_stabby.rs`）同步新增：**

```rust
#[stabby::stabby]
pub struct UploadInputCtxS {
    pub file_list: SVec<SArc<UploadFileDataS>>,
    pub config_info: SOption<SString>,
    pub extra_info: SOption<SString>,
    /// 活动目录（与 UploadInputCtx.work_dir 对应）
    pub work_dir: SOption<SString>,
}
```

> ABI 提示：stabby 结构体增字段属破坏性 ABI 变更，宿主与所有 dylib 插件须同版本重编。`uploader_example_plugin` 需重新编译。

### 2. 双向转换（`utils/ctx_util.rs`）

- `convert_input_ctx_s`：新增 `work_dir: input.work_dir.clone().map(Into::into).into()`。
- `convert_input_ctx`：从 stabby `work_dir` 还原 `Option<String>` 填入返回结构。

### 3. pipeline 透传（`file_uploader_core/src/pipeline/registry.rs`）

work_dir 是流程级常量，**仅从 source 透传，不取自 output**（`UploadOutputCtx` 不新增 work_dir 字段）：

- `output_to_input`（`registry.rs:162`）：构造下游 `UploadInputCtx` 时新增 `work_dir: source_ctx.work_dir.clone()`。
- 逐插件 `plugin_input` 构造（`registry.rs:222`）：新增 `work_dir: current_ctx.work_dir.clone()`。

> 设计依据：work_dir 为流程级单值，插件不创建新目录；与每插件不同的 `registry_config`（access 未来挂此处）正交。

### 4. SDK 新增工具（统一置于 `utils/`）

模块组织（`utils.rs`）：

```rust
pub mod ctx_util;     // 已有
pub mod config_util;  // 新增：配置读取
pub mod fs_util;      // 新增：work_dir 创建 + 沙箱流式 IO + 唯一文件名
```

#### 4.1 `utils/fs_util.rs`

```rust
use std::io::Read;
use std::path::{Path, PathBuf};

/// 默认文件名生成器：{timestamp_ms}_{hash}.{ext}
/// hash = DefaultHasher 对 (timestamp_ms, atomic_counter) 求哈希取低 64 位十六进制
pub fn gen_unique_name(ext: &str) -> String;

/// 创建活动目录：base/<id>，create_dir_all；返回完整路径供填入 ctx.work_dir
pub fn create_work_dir(base: &Path, id: &str) -> Result<PathBuf, UploadError>;

/// 沙箱核心：join(rel) → 词法规范化 → 校验仍以 work_dir 为前缀；
/// 越界（../、绝对路径）→ UploadError::WorkDirPathEscape。
/// 内部预留 check(work_dir, resolved) 扩展点（未来追加 access 收敛）。
pub fn resolve(work_dir: &str, rel: &str) -> Result<PathBuf, UploadError>;

// —— 写（流式 + 强制唯一名）——
/// 从任意 Read 流式拷贝（io::copy，分块）到 work_dir 内唯一名文件；
/// 返回 (文件名, 完整路径)。用默认 gen_unique_name。
pub fn write(work_dir: &str, ext: &str, reader: impl Read) -> Result<(String, PathBuf), UploadError>;

/// 同上，使用自定义生成器 gen（预留扩展点）。
pub fn write_with_gen(
    work_dir: &str,
    ext: &str,
    reader: impl Read,
    gen: fn(&str) -> String,
) -> Result<(String, PathBuf), UploadError>;

// —— 读（流式）——
/// 打开 work_dir 内文件，返回 BufReader<File>，调用方自行流式读取。
pub fn open_read(work_dir: &str, filename: &str)
    -> Result<std::io::BufReader<std::fs::File>, UploadError>;

// —— 读（便捷封装，基于 open_read）——
pub fn read_to_end(work_dir: &str, filename: &str) -> Result<Vec<u8>, UploadError>;
pub fn read_to_string(work_dir: &str, filename: &str) -> Result<String, UploadError>;

// —— 通用 ——
pub fn exists(work_dir: &str, rel: &str) -> bool;
pub fn create_dir(work_dir: &str, rel: &str) -> Result<(), UploadError>;
```

要点：
- 写入强制唯一名，调用方不再传文件名/路径（防覆盖、防逃逸语义）；`&[u8]` 可直接作为 `impl Read` 传入 `write`。
- 不提供「按任意 rel 写」的 `write` 变体；如未来确需指定名，另加 `write_named(work_dir, filename, reader)` 并复用 `resolve` 校验。
- 自定义生成器选函数指针 `fn(&str) -> String`（无状态、dylib 友好），非闭包/trait。
- **ABI 限制**：`BufReader<File>` 为 std 类型，仅限插件进程内使用，不可跨 dylib ABI 边界传递（非 stabby 类型）；插件通常自行读写、不跨 ABI 传流。

#### 4.2 `utils/config_util.rs`

从 `ctx.config_info: Arc<Option<Value>>` 读取 typed 值：

```rust
pub fn get_str(config: &Arc<Option<Value>>, key: &str) -> Option<String>;
pub fn get_bool(config: &Arc<Option<Value>>, key: &str) -> Option<bool>;
pub fn get_list(config: &Arc<Option<Value>>, key: &str) -> Vec<String>; // 数组→字符串列表
```

> 取代 `file_type_filter` 中手写的 `parse_list`。

#### 4.3 输出构造 helper（`UploadOutputCtx` 关联函数，`models/ctx.rs`）

无需新模块，直接在 `UploadOutputCtx` 上新增：

```rust
impl UploadOutputCtx {
    pub fn success(msg: impl Into<String>) -> Self;                                  // file_list=None
    pub fn success_files(msg: impl Into<String>, files: Vec<Arc<UploadFileData>>) -> Self;
    pub fn failed(msg: impl Into<String>) -> Self;
    pub fn interrupt(msg: impl Into<String>) -> Self;
}
```

### 5. 错误处理（`error.rs`）

`UploadError` 新增：

```rust
#[error("work_dir not set on context")]
WorkDirNotSet,

#[error("path '{path}' escapes work_dir '{work_dir}'")]
WorkDirPathEscape { work_dir: String, path: String },
```

底层 `std::io::Error` 复用现有 `CommonIoError(#[from])`，serde 复用 `JsonSerializeError`。

### 6. access 预留扩展点

`fs_util::resolve` 内部封装：

```rust
fn check(work_dir: &str, resolved: &Path) -> Result<(), UploadError> {
    // 本期：仅校验 resolved 是否以 work_dir 为前缀（归属判断）
    // 未来：在此追加 config.json access.fs_read/fs_write 白名单与 work_dir 的交集收敛
}
```

本期不实现 access 收敛逻辑，仅保证扩展点位置确定、签名稳定。

## 改造范围清单

新增字段导致所有 `UploadInputCtx { .. }` 字面量构造点编译必改（补 `work_dir`）：

- `file_uploader_core/src/pipeline/registry.rs`（含其 `#[cfg(test)]` 内多处）
- `file_uploader_plugins/src/pre_upload/file_type_filter.rs`（测试 `run()`）
- `file_uploader_core/src/main.rs`（示例入口）

`file_type_filter` 改造：
- `parse_list` → 改用 `config_util::get_list`。
- 输出构造 → 改用 `UploadOutputCtx::success / failed`。
- 测试 `run()` 构造 ctx 时补 `work_dir: None`。

## 同步更新：designing-in-process-plugins skill

新增能力须同步写入 `.agents/skills/designing-in-process-plugins/SKILL.md`，确保后续按该 skill 实现的插件默认采用新工具，避免继续手写样板或绕过沙箱。具体改动：

| SKILL 小节 | 改动 |
|---|---|
| 实现步骤·配置读取约定（第 3 节） | 引导改用 `file_uploader_sdk::utils::config_util::{get_str,get_bool,get_list}`，替代手写 `serde_json::Value` 解析 |
| 实现步骤·输出约定（第 4 节） | 引导改用 `UploadOutputCtx::{success, success_files, failed, interrupt}` 关联函数构造输出 |
| 实现步骤（新增小节）活动目录与文件 IO | 说明 `ctx.work_dir`（流程级常量、宿主预创建并透传、`None`=未设置）；插件读写中间文件须走 `file_uploader_sdk::utils::fs_util`（`write` 强制唯一名、`write_with_gen` 自定义生成器、`open_read` 流式、`read_to_end`/`read_to_string` 便捷、`resolve`/`create_work_dir`/`exists`/`create_dir`），**禁止直接用 `std::fs`**；列出限制（dylib 直接调 libc 不可拦截、`BufReader<File>` 不跨 dylib ABI） |
| 实现步骤·测试（第 6 节） | 构造 `UploadInputCtx` 时补 `work_dir` 字段（测试中通常 `None` 或临时目录） |
| 范例 | 注明 `file_type_filter` 已改造为使用 `config_util` + `UploadOutputCtx` 关联函数，作为新工具用法范例 |
| 常见错误 | 新增：`WorkDirNotSet`（`work_dir` 为 `None` 时调 `fs_util` IO）/ `WorkDirPathEscape`（`../` 或绝对路径越界）/ stabby 增字段致 dylib 须同版本重编 |
| 快速检查清单 | 增补四项：读配置走 `config_util`；构造输出走 `UploadOutputCtx` 关联函数；读写中间文件走 `fs_util`（不直接用 `std::fs`）；测试构造 `UploadInputCtx` 补 `work_dir` |

> 该 skill 改动作为实施计划中的一项独立任务，与代码改造一同提交。

## 测试策略

- **`fs_util`**：`create_work_dir` 创建成功；`resolve` 正常拼接与越界（`../` 逃逸、绝对路径）→ `WorkDirPathEscape`；`write` 返回名唯一且文件落在 work_dir 内、内容与输入一致（流式往返）；`write_with_gen` 使用自定义函数命名；`open_read` / `read_to_end` / `read_to_string` 往返；`gen_unique_name` 连续调用唯一性；`exists` / `create_dir`。
- **`config_util`**：`get_str/get_bool/get_list` 正常、key 缺失返回空/None、类型不符返回空/None。
- **`UploadOutputCtx` 关联函数**：四构造器 `result` 与字段正确性。
- **`ctx_util`**：含 `work_dir` 的 `UploadInputCtx ↔ UploadInputCtxS` 往返一致。
- **`registry`**：`output_to_input` 与逐插件 `plugin_input` 透传 `work_dir`（补现有测试构造点字段，新增断言 work_dir 一路透传）。
- **`file_type_filter`**：改造后既有过滤行为不变（回归）。

## ABI 兼容性

- `UploadInputCtxS` 增字段为破坏性 ABI 变更：宿主与所有 dylib 插件须同版本重编。
- `uploader_example_plugin` 需重新编译部署。
- `BufReader<File>` 等 std 类型仅限进程内使用，不跨 ABI 边界。

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| dylib 直接调 `std::fs` 绕过沙箱 | 文档明确强制力边界；未来 access 收敛可叠加审计/告警 |
| `canonicalize` 要求路径存在，写入校验失败 | 写入用词法规范化（components 拼接、不依赖 canonicalize），读取可 canonicalize |
| 唯一名并发冲突 | `gen_unique_name` 含原子计数器 + 时间戳 + hash，单进程内唯一 |
| 字段新增引发大量构造点编译错误 | 改造清单已穷举；`#[serde(default)]` 保证旧 JSON 反序列化兼容 |
