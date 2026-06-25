# 进程内插件迭代设计：upload_file_filter 重构 + default_input_handler 新增

- 日期：2026-06-19
- 范围：`file_uploader_plugins`（进程内插件）、`file_uploader_sdk`（工具层）
- 状态：待实现

## 1. 背景与目标

当前进程内插件存在两处不足：

1. `file_type_filter`（`file_uploader_plugins/src/pre_upload/file_type_filter.rs`）仅按上游传入的 `file_type`（MIME）做 glob 过滤。上游 MIME 不可靠（可被伪造、可能缺失），导致过滤结果不可信；且缺少对文件大小、文件名的过滤维度。
2. 缺少 `Input` 阶段（`UploadPhase::Input`）的内置插件，外部文件输入（本地路径 / 网络路径）在被过滤之前没有被引入工作目录、也没有被识别出可靠的类型，下游插件难以在统一的 `work_dir` 沙箱内处理。

目标：

- 重构过滤器为 `upload_file_filter`，提供「类型(MIME) + 名称 + 大小」三维度过滤，且消费由 Input 阶段嗅探得到的可靠 MIME。
- 新增 `default_input_handler`（Input 阶段，单插件），承担「把外部文件引入 work_dir 沙箱 + 魔数嗅探填充 `file_type`」的上下文补充职责；该插件在一个流程中必须最先执行。

## 2. 关键决策（brainstorming 已确认）

| 决策点 | 结论 |
|--------|------|
| `file_type` 字段内容来源 | 由 `default_input_handler` 用魔数嗅探（`infer` crate）填充，存 **MIME**（如 `image/png`） |
| 过滤器结构 | 单插件 `upload_file_filter`，三维度可选配置（某维度未配置则跳过） |
| 名称过滤维度 | 仅白名单 `pass_name`（glob），不设 `reject_name` |
| 大小过滤维度 | 仅上限 `max_size`，支持带单位字符串（`1kb`/`1mb`/`1g` 等，二进制 1024 进制）；`0`/缺省=不限 |
| 类型过滤维度 | `pass_type` / `reject_type`（MIME glob），表单预置常见 MIME 选项 |
| input 插件结构 | 单插件内聚，按 `data_type` 分流（本地 / 网络 / 二进制） |
| 路径语义 | 缓存 / 下载后 `input_path` 更新为 work_dir 内副本路径 |
| 读外部文件 | `fs_util` 提供「外部 → 沙箱」受控入口，不打破「IO 唯一出口」原则 |
| Binary 处理 | 当前不处理（原样透传）；字段预留，后续避免内存数据在插件间传输 |
| 容错 | 任一文件处理失败即整体 `Failed` 中断（不提供「跳过」选项） |

## 3. 架构总览

```
UploadPhase::Input                  UploadPhase::PreUpload
┌────────────────────────┐          ┌────────────────────────────┐
│  default_input_handler (新增)   │  file_   │  upload_file_filter (重构)  │
│  必须最先执行            │  type→   │                            │
│  · FilePath:  嗅探+缓存 │  填充    │  · 类型(MIME glob) pass/    │
│  · Network:   下载+嗅探 │  MIME    │    reject                  │
│  · Binary:    透传      │          │  · 名称(name glob) pass_name│
│  · 嗅探→file_type       │          │  · 大小 max_size           │
│  · 缓存/下载→改input_path│          │  全部淘汰→Failed           │
└────────────────────────┘          └────────────────────────────┘
        work_dir 沙箱内统一可读 ↑              ↑ 消费可靠 file_type
```

`execute_pipeline`（`file_uploader_core/src/pipeline/registry.rs:186`）已按 `Input → PreUpload → Upload → PostUpload → Output` 顺序执行，Input 阶段排序权重为 0（最先），无需改动执行引擎。

## 4. 插件 A：`upload_file_filter`（重构）

### 4.1 改名迁移

`file_type_filter` → `upload_file_filter`，以下位置同步：

- 资源目录：`file_uploader_plugins/resources/pre/file_type_filter/` → `upload_file_filter/`（`meta.json` 的 `name` 字段同步；插件 id 自动生成为 `in_process_PreUpload_upload_file_filter`）
- 模块：`src/pre_upload/file_type_filter.rs` → `src/pre_upload/upload_file_filter.rs`，`src/pre_upload.rs` 中 `pub mod` 同步
- 测试中引用的资源路径同步

> `build.rs` 递归复制整棵 `resources/` 树到 `target/resources/`，且已配置 `cargo:rerun-if-changed=resources`，新增/改名后重新 build 即可生效。

### 4.2 三维度过滤逻辑

保留条件（三维度 AND，某维度未配置则视为该维度恒真）：

```
keep(f) = (pass_type 空 ∨ 命中任一 pass_type)
        ∧ (未命中任一 reject_type)
        ∧ (pass_name 空 ∨ 命中任一 pass_name)
        ∧ (max_size≤0 ∨ f.size ≤ max_size)
```

- 类型 / 名称均编译为 `glob::Pattern`（沿用现有 `glob` 依赖与编译逻辑：无效模式 warn 并跳过）
- 类型作用于 `f.file_type`（MIME）；名称作用于 `f.name`（含后缀，天然支持 `*.png`、`report-*`）
- 大小作用于 `f.size`（`usize`）

### 4.3 配置（`config.json` params）

| key | 默认 | form |
|-----|------|------|
| `pass_type` | `[]` | select，multiple+allow_custom，options 预置常见 MIME |
| `reject_type` | `[]` | select，multiple+allow_custom，options 同上 |
| `pass_name` | `[]` | select，multiple+allow_custom（options 留空，靠 allow_custom 输入 glob） |
| `max_size` | `"0"` | text（支持 `1kb`/`1mb`/`1g` 等带单位；`0`=不限） |

`pass_type` / `reject_type` 预置 options（`value` 取 MIME，可被 glob 直接匹配）：

```json
[
  {"label":"图片(image/*)","value":"image/*"},
  {"label":"PNG","value":"image/png"},
  {"label":"JPEG","value":"image/jpeg"},
  {"label":"GIF","value":"image/gif"},
  {"label":"PDF","value":"application/pdf"},
  {"label":"纯文本","value":"text/plain"},
  {"label":"视频(video/*)","value":"video/*"},
  {"label":"音频(audio/*)","value":"audio/*"},
  {"label":"ZIP 压缩包","value":"application/zip"},
  {"label":"JSON","value":"application/json"}
]
```

权限（`access`）：保持只读、无网络（`fs_read: false, fs_write: false, network: false`，纯内存过滤不触发 IO）。

### 4.4 输出约定

- 有文件通过 → `UploadOutputCtx::success_files(msg, filtered)`
- 全部被淘汰 → `UploadOutputCtx::failed(msg)`（中断 pipeline，沿用现有约定）

### 4.5 测试

- 类型维度：pass 命中 / reject 剔除 / 空=全过 / 非法 glob 跳过
- 名称维度：`*.png` 命中 / 空白名单=全过
- 大小维度：超限淘汰 / `max_size="0"`=不限 / 带单位解析（`"1mb"`=1048576、`"1g"`=1073741824、纯数字按字节）
- 组合：三维度同时生效
- 全部淘汰 → `Failed`
- `config_info=None` → 全部通过（各维度默认恒真）

## 5. 插件 B：`default_input_handler`（新增）

### 5.1 职责

把外部输入引入 `work_dir` 沙箱 + 魔数嗅探填充 `file_type`(MIME) + 必要时更新 `input_path` 与 `size`。

### 5.2 按 `data_type` 分流

| `data_type` | 处理 |
|-------------|------|
| `FilePath`（本地） | 读外部文件头嗅探；若 `cache_local=true` → 拷贝进 work_dir、`input_path` 改副本路径、`size` 回填；若 `=false` → `input_path` 保留原值（仅完成嗅探） |
| `NetworkPath`（网络） | 若 `download_network=true` → HTTP 下载进 work_dir、`input_path` 改副本路径、读副本嗅探、`size` 回填；若 `=false` → 保留 URL、**不嗅探**（无法读内容，留给下游自定义下载） |
| `Binary`（内存） | **原样透传**，不做处理（字段预留；后续避免内存数据在插件间传输） |

> 嗅探顺序说明：本地文件无论是否缓存，都先读外部文件头嗅探（拷贝前后均可，统一在拷贝前读外部头）；网络文件必须下载后才能嗅探。

### 5.3 配置（`config.json` params）

| key | 默认 | form | 说明 |
|-----|------|------|------|
| `cache_local` | `true` | text(bool) | 本地文件是否缓存到 work_dir |
| `download_network` | `true` | text(bool) | 网络文件是否默认下载 |
| `sniff_type` | `true` | text(bool) | 是否执行魔数嗅探填充 `file_type` |

权限（`access`，当前纯透传不执行，仅声明意图）：`fs_read: true`（需读外部本地文件）、`fs_write: true`（需写 work_dir）、`network: true`（需下载）。

### 5.4 行为细节

- **嗅探**：读文件前 512 字节，用 `infer::get(&bytes)` 得 MIME；认不出 → 保留上游 `file_type` 原值 + `warn` 日志。
- **下载**：`reqwest::blocking` GET，将响应体作为 `impl Read` 流式写入 work_dir（直接复用现有 `fs_util::write(work_dir, ext, response_body_reader)`，该函数已强制唯一名、分块 `io::copy`）；URL 无效 / 网络错误 / 非 2xx → 返回 `UploadError` → 输出 `Failed`。
- **拷贝**：走 `fs_util::import_file`（见 §6）。
- **错误处理**：任一文件处理失败 → 立即 `UploadOutputCtx::failed(msg)` 中断 pipeline（不跳过）。
- **work_dir 缺失**：`ctx.work_dir` 为 `None` 且需写盘（缓存 / 下载）→ `UploadError::WorkDirNotSet` → `Failed`。

### 5.5 测试

- 本地文件：`cache_local=true` → 副本存在、`input_path` 指向副本、`size` 回填、`file_type` 被嗅探
- 本地文件：`cache_local=false` → `input_path` 不变、`file_type` 仍被嗅探
- 网络文件：`download_network=true` → 下载成功、`input_path` 改副本、嗅探成功（真实网络测试用 `#[ignore]` 标注，CI 默认跳过，本地手跑）
- 网络文件：`download_network=false` → URL 保留、`file_type` 不变
- Binary 文件：原样透传（`file_list` 含原项）
- 嗅探失败：未知格式 → `file_type` 保留原值
- 单文件失败（如本地路径不存在）→ `Failed` 中断
- `work_dir=None` 且需写盘 → `Failed`
- 模块注册：`src/input.rs` 新增 `pub mod default_input_handler;`，`src/lib.rs` 新增 `pub mod input;`

## 6. SDK 工具层改动（`file_uploader_sdk`）

### 6.1 `fs_util` 新增受控入口

当前 `fs_util` 的 `resolve` 强制所有相对路径落在 `work_dir` 前缀内（沙箱）。`default_input_handler` 需读取沙箱**外部**的原始本地文件，为此新增显式受控函数（不放宽 `resolve`，而是提供命名清晰的「外部 → 沙箱」入口）：

```rust
/// 把外部绝对路径文件拷贝进 work_dir 唯一名文件。
/// 内部：std::fs 读 src_abs_path（外部源，不走 resolve 前缀校验）→ write(work_dir, ext, reader)。
/// work_dir 为空 → WorkDirNotSet；src 不存在 → IO 错误。
pub fn import_file(work_dir: &str, src_abs_path: &str, ext: &str)
    -> Result<(String, PathBuf), UploadError>

/// 读取外部文件前 n 字节（供魔数嗅探）。不走 work_dir 沙箱校验。
pub fn read_external_head(src_abs_path: &str, n: usize) -> Result<Vec<u8>, UploadError>
```

- `import_file` 用于 `cache_local=true` 的本地缓存
- `read_external_head` 用于本地文件的嗅探（无论是否缓存）
- 两者均在函数名上体现「external」，明确这是受控的边界 IO，不破坏 `resolve` 对其他插件的保护

### 6.2 `config_util` 新增大小解析

```rust
/// 解析带单位的大小字符串为字节数。
/// 支持：纯数字（按字节）或 数字+单位，单位大小写不敏感。
///   b / byte → 1；k / kb → 1024；m / mb → 1024²；g / gb → 1024³；t / tb → 1024⁴
/// 非法输入 → None。
pub fn parse_size(s: &str) -> Option<u64>

/// 读取某 key 的字符串并用 parse_size 解析为字节数（max_size 用）。
/// 缺失/类型不符/无法解析 → None。
pub fn get_size(config: &Arc<Option<Value>>, key: &str) -> Option<u64>
```

### 6.3 数据模型与 ABI 影响

- **无字段变更**：`UploadFileData.file_type` 仍为 `String`，仅内容契约由「上游任意填」改为「input 阶段魔数嗅探的 MIME」。
- **不破坏 stabby ABI**：`SString` 布局不变，宿主与 dylib 无需因结构变更重编（仅语义约定更新，需在 `ctx.rs` / `ctx_stabby.rs` 注释中标注「file_type 存嗅探得到的 MIME，由 default_input_handler 填充」）。

## 7. 依赖变更

均新增到 `file_uploader_plugins/Cargo.toml`（保持 SDK 轻量）：

| 依赖 | 用途 | feature |
|------|------|---------|
| `infer` | 魔数嗅探 → MIME | 默认 |
| `reqwest` | 网络文件下载 | `blocking` |

`glob` 已存在，沿用。SDK 不新增依赖。

## 8. 非目标（YAGNI）

- 不实现 `config.json` `access` 权限的执行拦截（仍纯透传）
- 不为 Binary 数据提供落盘 / 插件间传输（字段预留）
- 不实现下载进度回调、断点续传、超时配置（后续按需迭代）
- 不实现嗅探结果的缓存 / 多次嗅探去重
- 不修改 `execute_pipeline` 执行引擎与回调机制

## 9. 风险与对策

| 风险 | 对策 |
|------|------|
| `reqwest::blocking` 在 tokio runtime 内会 panic | 当前 pipeline 为纯同步执行，无 tokio；若未来宿主嵌入异步 runtime，需切换下载实现或 spawn 独立线程 |
| 真实网络测试不稳定 | 下载测试用 `#[ignore]` 标注，CI 跳过；核心逻辑用本地文件覆盖 |
| 嗅探需读文件头，大文件 IO 成本 | 仅读前 512 字节，成本可控 |
| 本地 `cache_local=false` 时下游读原路径受限 | 当前约定：原路径在沙箱外，下游走 `fs_util` 仍受沙箱约束；若下游需读，应由 input 缓存引入。文档明确该边界 |

## 10. 文件清单（实现时的改动面）

新增：
- `file_uploader_plugins/resources/input/default_input_handler/{meta.json,config.json}`
- `file_uploader_plugins/src/input.rs`、`file_uploader_plugins/src/input/default_input_handler.rs`

重构迁移：
- `resources/pre/file_type_filter/` → `resources/pre/upload_file_filter/`（`meta.json` name 同步）
- `src/pre_upload/file_type_filter.rs` → `src/pre_upload/upload_file_filter.rs`
- `src/pre_upload.rs`、`src/lib.rs`（新增 `pub mod input;`）

SDK 扩展：
- `file_uploader_sdk/src/utils/fs_util.rs`（`import_file`、`read_external_head` + 测试）
- `file_uploader_sdk/src/utils/config_util.rs`（`parse_size` / `get_size` + 测试）
- `file_uploader_sdk/src/models/ctx.rs` / `ctx_stabby.rs`（`file_type` 语义注释）

依赖：
- `file_uploader_plugins/Cargo.toml`（`infer`、`reqwest` blocking）

注册示例更新：
- `file_uploader_core/src/main.rs`（若注册了旧 `file_type_filter`，改用新名 + 注册 `default_input_handler`）
