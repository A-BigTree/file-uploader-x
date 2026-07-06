# UploadInputCtx 单文件化重构设计

## 背景

当前 `UploadInputCtx`（`file_uploader_sdk/src/models/ctx.rs:178`）与 `UploadOutputCtx`（`ctx.rs:197`）的文件承载字段为列表：

```rust
pub file_list: Vec<Arc<UploadFileData>>,                 // UploadInputCtx
pub file_list: Option<Vec<Arc<UploadFileData>>>,         // UploadOutputCtx
```

实际使用场景中，一次上传流程只处理**单个文件**（用户选择一个文件上传）。列表结构属于过度设计：

- `output_to_input` / 每插件的 `plugin_input` 都在 clone 一个绝大多数情况下长度为 1 的 `Vec`；
- 列表为空（`vec![]`）与单文件的语义边界模糊；
- stabby 层、转换层、测试构造点都为「列表」付出额外的 `iter().map().collect()` 与 `SVec` 重建成本。

## 目标

将 `UploadInputCtx` / `UploadOutputCtx` 的文件承载字段从 `Vec<Arc<UploadFileData>>` 收敛为 `Option<Arc<UploadFileData>>`（单文件、可空），消除列表层。同步改造 stabby 层、转换层、pipeline 流转、内置插件、示例插件、测试与文档。

## 范围与非目标

- **范围**：
  - `UploadInputCtx` / `UploadOutputCtx` 原生结构字段变更；
  - `UploadInputCtxS` / `UploadOutputCtxS` stabby 结构同步；
  - `ctx_util.rs` 双向转换逻辑；
  - `registry.rs` 的 `output_to_input` 与 `execute_pipeline` 流转；
  - `UploadOutputCtx` 关联函数（`success_files` → `success_file`）；
  - 内置插件 `default_input_handler`（去循环）、`upload_file_filter`（重命名+语义变更）；
  - `main.rs` / `uploader_example_plugin` 构造点；
  - 全部受影响测试、README、SKILL 文档。
- **非目标**（显式排除）：
  - 不动 `UploadFileData` 结构本身（含其 `data: Option<SArc<SVec<u8>>>` 字段）；
  - 不动 `UploadTaskCtx` / `UploadProcessCtx`；
  - 不改 pipeline 执行顺序、不改 `PipelineCallback` 协议；
  - 不引入新字段、不改 ABI 以外的接口契约。

## 关键决策（已确认）

| 维度 | 决策 |
|------|------|
| 字段类型 | `file: Option<Arc<UploadFileData>>`（保留 `Arc`，clone 便宜、与现状一致） |
| 可空性 | `Option`（保留「无文件产出」语义，如纯日志插件、`success(msg)` / `failed` / `interrupt`） |
| 字段命名 | `file`（去掉 `file_list` 的 `_list` 后缀） |
| `filter` 插件去向 | 改名为校验器 `upload_file_validator` 并保留：从「批量 filter，全淘汰才 Failed」变为「单文件 accept/reject 校验」 |
| stabby 层 | 同步改 `file: SOption<SArc<UploadFileDataS>>`，保证 dylib ABI 一致 |
| 实现方案 | 方案 A（保留 `Arc`）：改动聚焦于去 `Vec` 层，`convert_*` 的 `SArc::new` 包裹逻辑保留 |

## 详细设计

### 1. 数据结构变更（`file_uploader_sdk/src/models/ctx.rs`）

```rust
pub struct UploadInputCtx {
    pub file: Option<Arc<UploadFileData>>,                // 原 file_list: Vec<Arc<UploadFileData>>
    pub config_info: Arc<Option<Value>>,
    pub extra_info: Option<HashMap<String, String>>,
    #[serde(skip)]
    pub related_process_info: Option<Weak<UploadProcessCtx>>,
    #[serde(default)]
    pub work_dir: Option<String>,
}

pub struct UploadOutputCtx {
    pub result: OutputResultType,
    pub message: String,
    pub file: Option<Arc<UploadFileData>>,                // 原 file_list: Option<Vec<Arc<UploadFileData>>>
    pub extra_info: Option<HashMap<String, String>>,
}
```

### 2. Stabby ABI 层（`file_uploader_sdk/src/models/ctx_stabby.rs`）

```rust
#[stabby::stabby]
pub struct UploadInputCtxS {
    pub file: SOption<SArc<UploadFileDataS>>,             // 原 file_list: SVec<SArc<UploadFileDataS>>
    pub config_info: SOption<SString>,
    pub extra_info: SOption<SString>,
    pub work_dir: SOption<SString>,
}

#[stabby::stabby]
pub struct UploadOutputCtxS {
    pub result: OutputResultType,
    pub message: SString,
    pub file: SOption<SArc<UploadFileDataS>>,             // 原 file_list: SOption<SVec<...>>
    pub extra_info: SOption<SString>,
}
```

`UploadFileDataS` 不变。

### 3. 转换层（`file_uploader_sdk/src/utils/ctx_util.rs`）

- `convert_input_ctx_s`：去掉 `file_list` 的 `iter().map(SArc::new(...)).collect::<SVec<_>>()`，改为对 `input.file` 直接 `match`：
  - `Some(arc) => SOption::Some(SArc::new(convert_file_data_s(arc)))`
  - `None => SOption::None()`
- `convert_input_ctx`：去掉 `input.file_list.iter().map(Arc::new(...)).collect::<Vec<_>>()`，改为 `input.file.match_ref(|f| Some(Arc::new(convert_file_data(f))), || None)`。
- `convert_output_ctx`：去掉 `SVec` 重建，同 `convert_input_ctx` 模式处理 `input.file`。
- `convert_file_data` / `convert_file_data_s` 不变。

### 4. `UploadOutputCtx` 关联函数（`ctx.rs`）

| 原签名 | 新签名 |
|---|---|
| `success(msg)` | 不变（`file: None`） |
| `success_files(msg, files: Vec<Arc<UploadFileData>>)` | `success_file(msg, file: Arc<UploadFileData>)` |
| `failed(msg)` | 不变 |
| `interrupt(msg)` | 不变 |

`success_file` 内部 `file: Some(file)`，其余字段同 `success`。

### 5. Pipeline 流转（`file_uploader_core/src/pipeline/registry.rs`）

- **`output_to_input`（`:162`）**：
  - `file_list: output.file_list.clone().unwrap_or_default()` → `file: output.file.clone()`
- **`execute_pipeline` 每插件 `plugin_input`（`:223`）**：
  - `file_list: current_ctx.file_list.clone()` → `file: current_ctx.file.clone()`
- **错误兜底 `fail_ctx`（`:246`）**：
  - `file_list: None` → `file: None`
- **末尾兜底返回（`:307`）**：
  - `file_list: Some(current_ctx.file_list)` → `file: current_ctx.file`
  - 注意：原 `current_ctx.file_list` 为 `Vec`，`Some(...)` 包裹；新 `current_ctx.file` 已是 `Option<Arc<...>>`，直接移动（`current_ctx` 后续不再使用）。

### 6. 插件改造

#### 6.1 `default_input_handler.rs`（Input 阶段）

- `run` 返回类型：`Result<Vec<Arc<UploadFileData>>, UploadError>` → `Result<Option<Arc<UploadFileData>>, UploadError>`。
- 去掉 `let mut out = Vec::with_capacity(ctx.file_list.len()); for f in &ctx.file_list { ... out.push(Arc::new(nf)); }`，改为：
  ```rust
  match ctx.file.as_ref() {
      None => Ok(None),
      Some(f) => {
          let mut nf = (**f).clone();
          // 原有 match nf.data_type { ... } 处理
          Ok(Some(Arc::new(nf)))
      }
  }
  ```
- `execute`：`UploadOutputCtx::success_files(msg, files)` → `UploadOutputCtx::success_file(msg, file)`，其中 `file` 由 `run` 返回的 `Option<Arc<_>>` 解包；`None` 时改用 `success(msg)`（无文件产出）。
- `message` 文案中的 `processed {} file(s)` 调整为单文件语境。

#### 6.2 `upload_file_filter` → `upload_file_validator`（PreUpload 阶段）

**重命名清单（全量）：**

| 项 | 原 | 新 |
|---|---|---|
| 文件路径 | `file_uploader_plugins/src/pre_upload/upload_file_filter.rs` | `upload_file_validator.rs` |
| struct | `UploadFileFilter` | `UploadFileValidator` |
| `name()` 返回 | `"upload_file_filter"` | `"upload_file_validator"` |
| 资源目录 | `file_uploader_plugins/resources/pre/upload_file_filter/` | `file_uploader_plugins/resources/pre/upload_file_validator/` |
| `meta.json` 的 `name` 字段 | `"upload_file_filter"` | `"upload_file_validator"` |
| `file_uploader_plugins/src/pre_upload/mod.rs` 模块声明 | `upload_file_filter` | `upload_file_validator` |
| `lib.rs` / 注册点引用 | 同步 | 同步 |

**语义变更：**

- 原「批量 filter，全部淘汰才 `Failed`」→ 新「单文件校验，不满足条件即 `Failed`，满足则透传该文件」。
- `keep()` 三维判定函数（pass_type / reject_type / pass_name / max_size）**逻辑保留**，但作用于 `ctx.file` 单个文件：
  ```rust
  fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
      let file = match ctx.file.as_ref() {
          Some(f) => f,
          None => return UploadOutputCtx::failed("upload_file_validator: no file"),
      };
      let (pass_type_raw, reject_type_raw, pass_name_raw, max_size) = parse_config(&ctx.config_info);
      let pass_type = compile_patterns(&pass_type_raw);
      let reject_type = compile_patterns(&reject_type_raw);
      let pass_name = compile_patterns(&pass_name_raw);
      if keep(file, &pass_type, &reject_type, &pass_name, max_size) {
          UploadOutputCtx::success_file(
              format!("upload_file_validator: accepted"),
              file.clone(),
          )
      } else {
          UploadOutputCtx::failed(format!(
              "upload_file_validator: rejected (pass_type={:?}, reject_type={:?}, pass_name={:?}, max_size={:?})",
              pass_type_raw, reject_type_raw, pass_name_raw, max_size,
          ))
      }
  }
  ```

**资源目录迁移：** 移动 `upload_file_filter/` 整目录到 `upload_file_validator/`（含 meta.json + config.json），更新 meta.json 的 `name`。`build.rs` 的递归复制逻辑无需改动（按目录扫描）。

#### 6.3 `main.rs`（`file_uploader_core/src/main.rs`）

- `file_list: vec![]` → `file: None`（或 `file: Some(Arc::new(...))` 若示例要演示单文件）。

#### 6.4 `uploader_example_plugin/src/lib.rs`

- `UploadOutputCtxS { file_list: stabby::option::Option::None(), ... }` → `file: stabby::option::Option::None()`。

### 7. 测试改写

#### 7.1 `ctx.rs::output_helper_tests`
- `success_files("done", vec![f])` → `success_file("done", f)`；
- 断言 `o.file_list.as_ref().unwrap().len() == 1` / `[0].name` → `o.file.as_ref().unwrap().name`。

#### 7.2 `ctx_util.rs::work_dir_tests`
- 构造 `UploadInputCtx { file_list: vec![Arc::new(...)], ... }` → `file: Some(Arc::new(...))`。

#### 7.3 `default_input_handler.rs::tests`
- `run(files: Vec<Arc<_>>, ...)` → `run(file: Option<Arc<_>>, ...)`；
- `run(vec![f], cfg, wd)` → `run(Some(f), cfg, wd)`；
- 断言 `out.file_list.as_ref().unwrap()` / `[0].xxx` → `out.file.as_ref().unwrap().xxx`。

#### 7.4 `upload_file_validator.rs::tests`（原 `upload_file_filter.rs::tests`）
- `run(files: Vec<_>, config)` → `run(file: Option<Arc<_>>, config)`；
- **删除多文件批量断言用例**（如 `pass_name_empty_passes_all` 断言 `len==2`、`combine_all_three_dims` 的多文件混合场景）；
- 改写为单文件 accept/reject 用例：每个用例传入 `Some(file)`，断言 `Success`/`Failed` 与（成功时）`out.file` 内容。
- 保留对 `pass_type` / `reject_type` / `pass_name` / `max_size` 四维配置的单文件覆盖。

#### 7.5 `registry.rs::tests`
- 所有 `UploadInputCtx { file_list: ..., ... }` 构造点（`:606`/`:732`/`:791`/`:801`/`:821`/`:882`/`:958`/`:988`/`:1023` 等）→ `file: ...`；
- MockPlugin `execute` 返回的 `UploadOutputCtx { file_list: None, ... }` → `file: None`；
- 涉及 `output.file_list.clone().unwrap_or_default()` 的断言逻辑同步。

### 8. 文档同步

- **`README.md`**：示例代码 `:133`/`:138`/`:194`/`:201` 的 `file_list` → `file`，`success_files` → `success_file`。
- **`AGENTS.md`**：「核心数据流」与「Pipeline 执行」段中对 `file_list` 的描述同步为 `file`。
- **`.agents/skills/designing-in-process-plugins/SKILL.md`**：`:74`「处理 `ctx.file_list`」→「处理 `ctx.file`」；`:99` `success_files` → `success_file`；`:124` 测试构造描述同步；`:149` 参考插件名 `file_type_filter` → `upload_file_validator`。

## 影响范围清单

| 文件 | 变更类型 |
|---|---|
| `file_uploader_sdk/src/models/ctx.rs` | 改字段 + 关联函数 + 测试 |
| `file_uploader_sdk/src/models/ctx_stabby.rs` | 改字段 |
| `file_uploader_sdk/src/utils/ctx_util.rs` | 改转换逻辑 + 测试 |
| `file_uploader_core/src/pipeline/registry.rs` | 改流转 + 测试 |
| `file_uploader_plugins/src/input/default_input_handler.rs` | 去循环 + 改输出 + 测试 |
| `file_uploader_plugins/src/pre_upload/upload_file_filter.rs` → `upload_file_validator.rs` | 重命名 + 语义变更 + 测试 |
| `file_uploader_plugins/src/pre_upload/mod.rs` | 模块声明改名 |
| `file_uploader_plugins/resources/pre/upload_file_filter/` → `upload_file_validator/` | 目录迁移 + meta.json |
| `file_uploader_core/src/main.rs` | 构造点 |
| `uploader_example_plugin/src/lib.rs` | 构造点 |
| `README.md` | 示例同步 |
| `AGENTS.md` | 描述同步 |
| `.agents/skills/designing-in-process-plugins/SKILL.md` | 描述同步 |

## 验证

- `cargo build`（全 workspace，含 dylib）通过；
- `cargo test`（含 `ctx_util` roundtrip、`default_input_handler`、`upload_file_validator`、`registry` pipeline 流转）全绿；
- `cargo run -p file_uploader_core` 示例运行无 panic。
