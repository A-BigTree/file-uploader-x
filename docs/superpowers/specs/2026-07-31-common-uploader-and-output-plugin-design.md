# 通用上传插件与通用输出插件 设计文档

- 日期：2026-07-31
- 分支：`feature/complete_common_plugin`
- 状态：待评审

## 1. 目标

在 `file_uploader_plugins` 中补齐两个进程内插件，填上目前空缺的 Upload 与 Output 阶段：

1. **通用上传插件 `common_uploader`**（Upload 阶段）——支持多存储供应商，本期实现 Cloudflare R2；不同供应商的入参用 `config.json` 的 `groups` 互斥隔离。
2. **通用输出插件 `common_output`**（Output 阶段）——将上传结果渲染为 markdown、HTML 或纯超链接三种格式之一。

### 非目标

- 不实现 R2 之外的供应商（S3 / OSS / COS 留作后续，本期只保证扩展点就位）。
- 不引入异步运行时，不引入 `aws-sdk-s3`。
- 不做断点续传（分片状态不持久化，进程重启后从头开始）。
- 不做分片并发上传。

## 2. 关键约束

| 约束 | 来源 | 影响 |
|---|---|---|
| `UploadPlugin::execute` 为同步方法 `fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx` | `file_uploader_sdk/src/models/interface.rs` | 不能用 async SDK；沿用 `reqwest::blocking` |
| workspace 无 tokio、无 aws-sdk | 根 `Cargo.toml` | R2 签名自研 |
| `execute` 无法返回 `Result`，错误须转 `UploadOutputCtx::failed` | 同上 | 逻辑放私有 `run()` 返回 `Result`，`execute` 只做转译 |
| 插件读写文件必须走 `fs_util`，禁止 `std::fs` | `docs/references/plugin-specification.md` §6.8 | 分片读取用 `fs_util::open_read` |
| `groups` 非空时运行态配置必须带 `group` | 规范 §3.3 | R2 参数置于 `group: "r2"` 下 |
| `output_to_input` 用 `output.file.clone()` | `file_uploader_core/src/pipeline/registry.rs` | 上传插件必须回传 `file`，否则下游丢失文件信息 |
| `extra_info` 值类型只能是 `String` | `UploadOutputCtx` | 辅助信息全部以字符串形式传递 |

## 3. 架构

### 3.1 模块结构

```
file_uploader_plugins/src/
├── lib.rs                       // 新增 pub mod output;
├── upload.rs                    // 由空文件改为 pub mod common_uploader;
├── upload/
│   ├── common_uploader.rs       // 插件入口：group 分派 + ctx 编解码
│   ├── provider.rs              // trait StorageProvider + UploadRequest / UploadedObject
│   ├── sigv4.rs                 // AWS SigV4 签名（S3 系共用，纯函数）
│   ├── naming.rs                // object_key 生成（纯函数）
│   ├── multipart.rs             // S3 Multipart 四步编排 + XML 拼装/提取
│   └── r2.rs                    // impl StorageProvider for R2Provider
├── output.rs                    // 新建：pub mod common_output;
└── output/
    ├── common_output.rs         // 插件入口：读 file + format 分派
    └── render.rs                // markdown / html / link 渲染 + 转义 + 模板（纯函数）
```

资源目录（build.rs 整树自动复制，无需改动构建脚本）：

```
file_uploader_plugins/resources/
├── upload/common_uploader/{meta.json, config.json, README.md}
└── output/common_output/{meta.json, config.json, README.md}
```

Output 阶段的目录段名此前规范未约定，本设计确定为 `output`，与既有 `input` / `pre` 保持同一风格（枚举名小写、PreUpload 简写为 pre）。

### 3.2 供应商抽象

```rust
pub struct UploadRequest<'a> {
    pub work_dir: &'a str,
    pub local_path: &'a str,   // work_dir 内的文件路径
    pub object_key: &'a str,   // 已由 naming 算好
    pub content_type: &'a str,
    pub size: usize,
}

pub struct UploadedObject {
    pub url: String,
    pub object_key: String,
}

pub trait StorageProvider {
    fn provider_id(&self) -> &'static str;
    fn upload(&self, req: &UploadRequest) -> Result<UploadedObject, UploadError>;
}
```

职责边界：provider 只负责「把字节送上去、给回可访问 URL」。命名策略、重试次数、分片选路、ctx 编解码都在插件主体或共用模块，不下沉到 provider——这样新增供应商只需实现一个 `upload`。

分派：

```rust
let provider: Box<dyn StorageProvider> = match config_util::get_group(&ctx.config_info).as_deref() {
    Some("r2") => Box::new(R2Provider::from_config(&ctx.config_info)?),
    other => return Err(UploadError::PluginParamInvalid(
        format!("common_uploader: 不支持的分组 {other:?}"))),
};
```

## 4. 上传插件 `common_uploader`

### 4.1 meta.json

```json
{
  "name": "common_uploader",
  "title": "通用上传插件",
  "description": "将工作目录中的文件上传至对象存储，当前支持 Cloudflare R2",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "Upload"
}
```

### 4.2 config.json

`access`：`{ "fs_read": true, "fs_write": false, "network": true }`

**common 段**（与供应商无关，新增分组即可复用）

| key | 控件 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `timeout_secs` | number 1–3600 integer | 否 | 30 | 单次 HTTP 请求超时 |
| `retry_times` | number 0–5 integer | 否 | 2 | 每片/单次 PUT 各自独立重试 |
| `naming` | select 单选 | 否 | `date_uuid` | `origin` / `uuid` / `hash` / `date_uuid` |
| `key_prefix` | text max_len 256 | 否 | `""` | 如 `uploads/` |
| `multipart_enabled` | switch | 否 | `true` | 关闭则一律单次 PUT |
| `multipart_part_size` | text | 否 | `"64MB"` | 走 `config_util::get_size`，支持 `64MB` / `5MiB` 写法 |

**group `r2` 段**

| key | 控件 | 必填 | 说明 |
|---|---|---|---|
| `account_id` | text min_len 1 | 是 | 拼 endpoint |
| `bucket` | text min_len 1 | 是 | 桶名 |
| `access_key_id` | text min_len 1 | 是 | R2 API Token 的 Access Key ID |
| `secret_access_key` | text **secret** min_len 1 | 是 | 密钥，前端不回显 |
| `public_base_url` | text min_len 1 | 是 | 公开访问域名，如 `https://cdn.example.com` |
| `overwrite` | switch | 否 | 默认 `true`；关闭时先 HEAD 探测，已存在则失败 |

`public_base_url` 定为必填：本插件的产出契约是「可访问 URL」，无域名则下游输出插件无法工作，早失败优于晚失败。

`multipart_part_size` 用 text 而非 number，是为了复用 `config_util::get_size` 支持带单位写法，比让用户填 `67108864` 友好。

### 4.3 object_key 生成

| naming | 形式 | 说明 |
|---|---|---|
| `origin` | `{prefix}{sanitized_name}` | 文件名清洗：路径分隔符、控制字符、`..` 全部替换为 `_` |
| `uuid` | `{prefix}{unique}.{ext}` | `unique` 复用 `fs_util::gen_unique_name`（形如 `{时间戳}_{随机hash}`），并非严格 RFC 4122 UUID；此举避免引入 uuid crate，README 中如实说明 |
| `hash` | `{prefix}{sha256[..16]}.{ext}` | 需完整读一遍文件算全文 sha256 |
| `date_uuid` | `{prefix}{YYYY}/{MM}/{DD}/{unique}.{ext}` | 默认；日期取本地时区当天，`unique` 同上 |

`ext` 取自原文件名后缀，无后缀则省略 `.` 部分。

### 4.4 执行流程

```
1. 前置校验：work_dir 非空、ctx.file 存在、本地文件可读
2. 选路（仅依赖 size，无需读文件）：
     !multipart_enabled || size <= part_size  → 单次 PUT
     否则                                      → Multipart
3. 若 单次PUT 或 naming == hash：流式计算全文 sha256（一次读取，两处复用）
     单次 PUT 需要它填 x-amz-content-sha256；Multipart 逐片单独算，不需要全文 hash
4. object_key = naming::object_key(strategy, prefix, file, full_hash_opt)
5. provider = R2Provider::from_config()   // 缺失/非法凭证 → failed
6. overwrite == false → HEAD 探测，已存在则 failed
7. 按第 2 步选定的路径执行上传
8. 输出装配
```

先选路后算 hash，是为了让 Multipart + 非 `hash` 命名的场景免去一次全文扫描。

**单次 PUT**

```
PUT https://{account_id}.r2.cloudflarestorage.com/{bucket}/{object_key}
Host, x-amz-date, x-amz-content-sha256: <全文 hash>, Content-Type: <file.file_type>
Authorization: AWS4-HMAC-SHA256 ...   // region = auto, service = s3
body: fs_util::open_read 流式
```

内存峰值 ≈ BufReader 缓冲（约 64 KB），与文件体积无关。

**Multipart（串行）**

```
POST  ?uploads                                   → 从 XML 取 <UploadId>
for part_number in 1..=N:
    读 part_size 字节入复用缓冲 → 该片 sha256 → 签名
    PUT ?partNumber={n}&uploadId={id}            → 从响应头取 ETag
    单片失败按 retry_times 重试（只重传该片）
POST ?uploadId={id}  + XML(PartNumber, ETag 列表) → Complete
任一步不可恢复失败 → DELETE ?uploadId={id} (Abort) 清理 → failed
```

内存峰值 ≈ `part_size`（默认 64 MiB）+ 缓冲；缓冲区跨片复用（`clear()` 而非重新分配）。

分片上限 10000（S3 协议硬限），故默认配置下单文件上限约 640 GiB。

**不引入 XML 库**：`CreateMultipartUpload` 只需提取 `<UploadId>`，字符串定位即可；`UploadPart` 的 ETag 在响应头而非 body；Complete 的请求体由字符串拼接生成（ETag 内容做 XML 实体转义）。

**重试策略**：仅对网络层错误与 HTTP 5xx 重试，4xx 不重试（凭证错、权限错重试无意义）。固定间隔，不做指数退避。

### 4.5 输出契约

成功时手写 `UploadOutputCtx` 字面量（`success_file` 不带 `extra_info`，不满足需要）：

```rust
UploadOutputCtx {
    result: OutputResultType::Success,
    message: format!("common_uploader: uploaded to {url}"),
    file: Some(Arc::new(UploadFileData {
        data_type: FileDataType::NetworkPath,   // 改写
        input_path: url.clone(),                // 改写为公网 URL
        id: old.id.clone(),                     // 不变
        name: old.name.clone(),                 // 不变
        file_type: old.file_type.clone(),       // 不变
        size: old.size,                         // 不变
        data: None,
    })),
    extra_info: Some(map![
        "upload_local_path" => 原 work_dir 内路径,
        "upload_object_key" => object_key,
        "upload_provider"   => "r2",
    ]),
}
```

**上传结果的主通道是 `file` 本身**：`data_type` 置为 `NetworkPath`、`input_path` 置为公网 URL，这是 `UploadFileData` 原生就支持的链接表达，下游无需约定额外 key 即可取用。`extra_info` 只承载辅助信息——原本地路径供 PostUpload 阶段清理临时文件或生成缩略图，`object_key` 供审计与后续删除。

### 4.6 validate_params

只做声明式 schema 表达不了的部分：

- `public_base_url` 必须以 `http://` 或 `https://` 开头，且无首尾空白。
- `key_prefix` 不得以 `/` 开头、不得包含 `..`。
- `naming` 取值必须在 `origin`/`uuid`/`hash`/`date_uuid` 之内。
- `multipart_part_size` 必须可被 `parse_size` 解析，且落在 5 MiB ~ 5 GiB（S3 对非末片的硬性下限与上限）。
- 分片数上限提示：若 `multipart_part_size` 过小导致理论分片数可能超 10000，报错并提示调大。

### 4.7 日志纪律

**绝不整体序列化 ctx**——本插件配置含 `secret_access_key`，`upload_file_validator` 里那种 `serde_json::to_string(ctx)` 写法会直接把密钥打进日志。只记录 `object_key`、`provider`、分片进度、HTTP 状态码、耗时。

## 5. 输出插件 `common_output`

### 5.1 meta.json

```json
{
  "name": "common_output",
  "title": "通用输出插件",
  "description": "将上传结果渲染为 markdown、HTML 或纯超链接",
  "version": "0.0.1",
  "author": "A-BigTree",
  "phase": "Output"
}
```

### 5.2 config.json

`access`：`{ "fs_read": false, "fs_write": false, "network": false }`，`groups: []`。

| key | 控件 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `format` | select 单选 `markdown`/`html`/`link` | 否 | `markdown` | |
| `template` | text max_len 512 | 否 | `""` | 非空则覆盖 `format`；占位符 `{url}` `{name}` `{size}` `{type}` |

### 5.3 渲染规则

| format | 图片（`file_type` 以 `image/` 开头） | 非图片 |
|---|---|---|
| `markdown` | `![name](url)` | `[name](url)` |
| `html` | `<img src="url" alt="name">` | `<a href="url">name</a>` |
| `link` | `url` | `url` |

**转义**

- HTML：对 `name` 与 `url` 转义 `&` `<` `>` `"` `'` 为实体。
- Markdown：对 `name` 中的 `[` `]` `(` `)` 加反斜杠；对 `url` 中的空格与括号做百分号编码。
- 模板模式：占位符替换后不做转义（用户自定义模板意味着自己掌控输出形态），此约定写入 README。

### 5.4 输入契约与错误

`ctx.file` 必须存在，且 `data_type == NetworkPath`、`input_path` 以 `http` 开头。否则：

```
failed("common_output: 未获得可访问 URL，请确认上游上传插件已执行并配置了访问域名")
```

不静默输出空链接——失败可见优于产出垃圾。

### 5.5 输出

```rust
UploadOutputCtx {
    result: Success,
    message: <渲染结果>,
    file: <原样回传>,
    extra_info: 原 extra_info + { "output": <渲染结果>, "output_format": <markdown|html|link|template> },
}
```

作为 pipeline 最后一环，本插件的 `message` 即 `execute_pipeline` 的返回值，调用方可直接取用。

## 6. 依赖变更

`file_uploader_plugins/Cargo.toml` 新增：

```toml
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
chrono = { workspace = true }
```

不引入 tokio、aws-sdk-s3、XML 解析库、uuid crate。

## 7. 测试策略

| 层次 | 覆盖内容 |
|---|---|
| `sigv4` 纯函数 | 用 AWS 官方测试向量校验规范请求串、待签字符串、最终签名 |
| `naming` 纯函数 | 四种策略 × prefix 拼接 × 无后缀文件 × 危险字符清洗（`../`、控制字符、路径分隔符）|
| `multipart` 纯函数 | Complete 的 XML 拼装（含 ETag 转义）；`<UploadId>` 提取（正常/缺失/畸形）；分片数与偏移计算（整除、有余数、`size == part_size` 边界）|
| `render` 纯函数 | 三格式 × 图片/非图片 × HTML 转义 × Markdown 转义 × 四个模板占位符 |
| 插件级 | `validate_params` 全部非法输入；缺 `group`；未知 `group`；缺凭证；选路（`multipart_enabled=false` 时大文件仍走单次 PUT；`size == part_size` 走单次 PUT）|
| 输出插件级 | 缺 `file`；`data_type` 非 `NetworkPath`；`input_path` 非 http；`format` 缺省取默认；`template` 优先于 `format` |
| 集成 `#[ignore]` | 真实 R2 单次上传与分片上传（构造 >64 MiB 临时文件）；凭证读环境变量，缺凭证时 `eprintln!` 后 return（沿用 `download_network_real` 范式）|

不修改 `file_uploader_core/src/main.rs`——本期只交付插件与单测。

## 8. 风险与取舍

1. **SigV4 自研是本次唯一高风险点**。缓解：签名逻辑与 HTTP 发送彻底分离，用 AWS 文档测试向量做单测，签名错误可离线复现。
2. **分片会提高内存峰值**（默认约 64 MiB），因为每片必须先算出 sha256 才能签名。单次 PUT 反而是流式的、峰值仅数十 KB。按阈值自动选路正是为了让多数中小文件走更省的单次 PUT 路径。
3. **Abort 本身可能失败**（网络已断），此时 R2 会残留未完成分片并计费。缓解：Abort 失败不吞掉，在 `message` 中带上 `uploadId` 与 `object_key`，便于人工或后续 PostUpload 插件清理。
4. **上游依赖**：本插件要求文件已在 `work_dir` 内，即需先执行 `default_input_handler`。此依赖写入 README 的「什么时候用它」。
5. **不做断点续传**：分片状态不落盘，进程重启需重新上传。
6. **`overwrite=false` 的 HEAD 探测存在竞态**：探测与 PUT 之间他方可能写入同一 key。R2 不支持条件写，此竞态本期不解决，README 中说明。

## 9. 交付清单

- [ ] `upload/{common_uploader,provider,sigv4,naming,multipart,r2}.rs`
- [ ] `output/{common_output,render}.rs`，`src/output.rs`，`lib.rs` 新增 `pub mod output;`
- [ ] `src/upload.rs` 由空文件改为模块声明
- [ ] `resources/upload/common_uploader/{meta.json,config.json,README.md}`
- [ ] `resources/output/common_output/{meta.json,config.json,README.md}`
- [ ] `Cargo.toml` 新增 4 个依赖
- [ ] 单测全绿；`docs/references/plugin-specification.md` 补充 `output` 目录段名约定
