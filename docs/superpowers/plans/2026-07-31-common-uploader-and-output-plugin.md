# 通用上传插件与通用输出插件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `file_uploader_plugins` 中实现支持 Cloudflare R2、默认按 64 MiB 阈值启用串行 Multipart 的通用上传插件，以及支持 markdown、HTML、超链接与自定义模板的通用输出插件。

**Architecture:** `common_uploader` 通过内部 `StorageProvider` trait 隔离供应商，R2 实现使用 `reqwest::blocking` 与自研 AWS SigV4，上传成功后将 `UploadFileData` 原生改写为 `NetworkPath`。`common_output` 只消费该 `UploadFileData`，以纯函数完成格式化和转义；配置使用现有 `common + groups` schema。

**Tech Stack:** Rust 2024、`reqwest::blocking`、`hmac`、`sha2`、`hex`、`chrono`、`serde_json`、现有 `file_uploader_sdk`。

## Global Constraints

- `UploadPlugin::execute` 保持同步；不得引入 tokio、`aws-sdk-s3`、XML 解析库或 uuid crate。
- 所有文件读取必须通过 `file_uploader_sdk::utils::fs_util`，不得直接用 `std::fs` 打开上传文件。
- R2 参数必须位于互斥的 `group: "r2"`；供应商无关参数位于 `common`。
- `multipart_enabled` 默认 `true`，`multipart_part_size` 默认 `"64MB"`；`size <= part_size` 走单次 PUT，只有更大的文件走串行 Multipart。
- Multipart 分片大小限制为 5 MiB 至 5 GiB，分片数不得超过 10000。
- 命名策略默认 `date_uuid`；这里的 `uuid` 表示现有 `fs_util::gen_unique_name` 生成的时间戳加随机 hash，不宣称符合 RFC 4122。
- 上传结果主通道必须是 `UploadFileData { data_type: NetworkPath, input_path: public_url }`；`extra_info` 仅补充本地路径、object key 和 provider。
- 禁止整体序列化或记录 `ctx`/配置，避免泄漏 `secret_access_key`。
- 不修改 `file_uploader_core/src/main.rs`。
- 不添加与本功能无关的重构或注释。

## File Map

**Create**

- `file_uploader_plugins/src/upload/common_uploader.rs`：UploadPlugin 入口、配置读取、供应商分派、输出 ctx 装配。
- `file_uploader_plugins/src/upload/common_uploader/provider.rs`：供应商接口与共用 DTO。
- `file_uploader_plugins/src/upload/common_uploader/naming.rs`：object key 生成、文件名清洗、扩展名处理。
- `file_uploader_plugins/src/upload/common_uploader/sigv4.rs`：AWS SigV4 规范化、签名和 URI 编码。
- `file_uploader_plugins/src/upload/common_uploader/multipart.rs`：Multipart XML 与分片边界纯函数。
- `file_uploader_plugins/src/upload/common_uploader/r2.rs`：R2 配置、HEAD、单次 PUT、Multipart HTTP 流程与重试。
- `file_uploader_plugins/src/output.rs`：Output 模块声明。
- `file_uploader_plugins/src/output/common_output.rs`：OutputPlugin 入口、输入契约、输出 ctx 装配。
- `file_uploader_plugins/src/output/common_output/render.rs`：三种格式、模板与转义纯函数。
- `file_uploader_plugins/resources/upload/common_uploader/meta.json`
- `file_uploader_plugins/resources/upload/common_uploader/config.json`
- `file_uploader_plugins/resources/upload/common_uploader/README.md`
- `file_uploader_plugins/resources/output/common_output/meta.json`
- `file_uploader_plugins/resources/output/common_output/config.json`
- `file_uploader_plugins/resources/output/common_output/README.md`

**Modify**

- `file_uploader_plugins/Cargo.toml`：新增四个轻依赖。
- `file_uploader_plugins/src/upload.rs`：导出 `common_uploader`。
- `file_uploader_plugins/src/lib.rs`：导出 `output`。
- `docs/references/plugin-specification.md`：补充 Output 资源目录段名与两个插件间的 file 契约。

---

### Task 1: 模块骨架与供应商接口

**Files:**
- Modify: `file_uploader_plugins/Cargo.toml`
- Modify: `file_uploader_plugins/src/upload.rs`
- Modify: `file_uploader_plugins/src/lib.rs`
- Create: `file_uploader_plugins/src/upload/common_uploader.rs`
- Create: `file_uploader_plugins/src/upload/common_uploader/provider.rs`
- Create: `file_uploader_plugins/src/output.rs`
- Create: `file_uploader_plugins/src/output/common_output.rs`

**Interfaces:**
- Produces: `StorageProvider::upload(&UploadRequest) -> Result<UploadedObject, UploadError>`，后续 R2 与插件入口共同依赖。
- Produces: `UploadRequest` 中包含本地文件、object key、内容类型、大小、全文 hash 与 multipart 选项。

- [ ] **Step 1: 添加依赖并声明模块**

将 `file_uploader_plugins/Cargo.toml` 的 `[dependencies]` 追加：

```toml
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
chrono = {workspace = true}
```

将 `src/upload.rs` 改为：

```rust
pub mod common_uploader;
```

在 `src/lib.rs` 末尾新增：

```rust
pub mod output;
```

创建 `src/output.rs`：

```rust
pub mod common_output;
```

- [ ] **Step 2: 定义 provider 共用类型并写编译测试**

创建 `provider.rs`：

```rust
use file_uploader_sdk::error::UploadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadMode {
    Single,
    Multipart { part_size: u64 },
}

pub struct UploadRequest<'a> {
    pub work_dir: &'a str,
    pub local_path: &'a str,
    pub object_key: &'a str,
    pub content_type: &'a str,
    pub size: u64,
    pub full_hash: Option<&'a str>,
    pub mode: UploadMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedObject {
    pub url: String,
    pub object_key: String,
}

pub trait StorageProvider {
    fn provider_id(&self) -> &'static str;
    fn upload(&self, request: &UploadRequest<'_>) -> Result<UploadedObject, UploadError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_mode_carries_part_size() {
        assert_eq!(
            UploadMode::Multipart { part_size: 64 * 1024 * 1024 },
            UploadMode::Multipart { part_size: 64 * 1024 * 1024 }
        );
    }
}
```

- [ ] **Step 3: 创建可编译的插件入口骨架**

创建 `common_uploader.rs`：

```rust
mod naming;
mod multipart;
mod provider;
mod r2;

pub struct CommonUploader;
```

创建 `common_output.rs`：

```rust
mod render;

pub struct CommonOutput;
```

同时创建空的 `naming.rs`、`multipart.rs`、`r2.rs`、`render.rs`，使模块解析成功。完整 trait 实现分别在 Task 6 和 Task 8 一次性加入，避免提交临时占位行为。

- [ ] **Step 4: 运行编译与目标测试**

Run: `cargo test -p file_uploader_plugins upload_mode_carries_part_size`

Expected: PASS。

Run: `cargo check -p file_uploader_plugins`

Expected: PASS；允许后续任务尚未使用的 dead-code warning，但不允许 error。

- [ ] **Step 5: 提交**

```bash
git add file_uploader_plugins/Cargo.toml file_uploader_plugins/src
git commit -m "feat(plugins): 搭建通用上传与输出插件模块"
```

---

### Task 2: object key 命名与全文 hash

**Files:**
- Modify: `file_uploader_plugins/src/upload/common_uploader/naming.rs`

**Interfaces:**
- Consumes: `fs_util::gen_unique_name(ext)`。
- Produces: `NamingStrategy::parse`、`sha256_file`、`build_object_key`，供 `common_uploader` 使用。

- [ ] **Step 1: 写命名与清洗的失败测试**

在 `naming.rs` 写测试，固定日期参数以保证确定性：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_origin_removes_path_and_parent_segments() {
        assert_eq!(sanitize_name("../a\\b\0.png"), "__a_b_.png");
    }

    #[test]
    fn origin_joins_normalized_prefix() {
        let key = build_object_key(
            NamingStrategy::Origin,
            "uploads/",
            "../demo.png",
            None,
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        ).unwrap();
        assert_eq!(key, "uploads/__demo.png");
    }

    #[test]
    fn date_uuid_uses_date_path_and_keeps_extension() {
        let key = build_object_key(
            NamingStrategy::DateUuid,
            "images/",
            "demo.png",
            None,
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        ).unwrap();
        assert!(key.starts_with("images/2026/07/31/"));
        assert!(key.ends_with(".png"));
    }

    #[test]
    fn hash_requires_and_truncates_full_hash() {
        let key = build_object_key(
            NamingStrategy::Hash,
            "",
            "demo.bin",
            Some("0123456789abcdefaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        ).unwrap();
        assert_eq!(key, "0123456789abcdef.bin");
    }

    #[test]
    fn parser_rejects_unknown_strategy() {
        assert!(NamingStrategy::parse("random").is_err());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins common_uploader::naming::tests`

Expected: FAIL，缺少 `NamingStrategy` / `build_object_key`。

- [ ] **Step 3: 实现命名函数**

实现以下精确接口：

```rust
use chrono::{Datelike, NaiveDate};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::utils::fs_util;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingStrategy { Origin, Uuid, Hash, DateUuid }

impl NamingStrategy {
    pub fn parse(value: &str) -> Result<Self, UploadError>;
    pub fn needs_full_hash(self) -> bool;
}

pub fn sha256_file(work_dir: &str, path: &str) -> Result<String, UploadError> {
    let mut reader = fs_util::open_read(work_dir, path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn sanitize_name(name: &str) -> String;

pub fn build_object_key(
    strategy: NamingStrategy,
    prefix: &str,
    original_name: &str,
    full_hash: Option<&str>,
    date: NaiveDate,
) -> Result<String, UploadError>;
```

实现规则：prefix 仅移除尾部重复 `/` 后再补一个 `/`；`origin` 清洗 `/`、`\\`、控制字符，并逐次将 `..` 替换为 `_`；`uuid` 和 `date_uuid` 调 `gen_unique_name(ext)`；`hash` 缺少或短于 16 字符时返回 `UploadError::InvalidFormat`。

- [ ] **Step 4: 增加流式 hash 测试**

用现有 `fs_util::create_work_dir` 与 `fs_util::write` 写入 `b"abc"`，断言：

```rust
assert_eq!(
    sha256_file(&work_dir, &path_string).unwrap(),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
);
```

测试结束删除临时目录。

- [ ] **Step 5: 运行测试并提交**

Run: `cargo test -p file_uploader_plugins common_uploader::naming::tests`

Expected: PASS。

```bash
git add file_uploader_plugins/src/upload/common_uploader/naming.rs
git commit -m "feat(upload): 实现对象命名与流式哈希"
```

---

### Task 3: AWS SigV4 签名

**Files:**
- Modify: `file_uploader_plugins/src/upload/common_uploader/sigv4.rs`

**Interfaces:**
- Produces: `Credentials`、`SigningInput`、`SignedHeaders`、`sign`、`encode_path`，供 R2 HTTP 请求调用。

- [ ] **Step 1: 写 AWS 官方向量测试**

使用 AWS IAM ListUsers 官方示例，输入：

```rust
let credentials = Credentials {
    access_key_id: "AKIDEXAMPLE".into(),
    secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
};
let input = SigningInput {
    method: "GET",
    canonical_uri: "/",
    canonical_query: "Action=ListUsers&Version=2010-05-08",
    host: "iam.amazonaws.com",
    content_type: None,
    payload_hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    timestamp: Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap(),
    region: "us-east-1",
    service: "iam",
};
let signed = sign(&credentials, &input).unwrap();
assert_eq!(signed.amz_date, "20150830T123600Z");
assert!(signed.authorization.starts_with(
    "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request"
));
assert!(signed.authorization.ends_with(
    "Signature=5d672d79c15b13162d9279b0855cfba67822e4b5b62229461c1d180a0f32b3d"
));
```

再写 URI 编码测试：

```rust
assert_eq!(encode_path("bucket/a b/中.png"), "/bucket/a%20b/%E4%B8%AD.png");
assert_eq!(canonical_query(&[("uploadId", "a+b/="), ("partNumber", "2")]),
           "partNumber=2&uploadId=a%2Bb%2F%3D");
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins common_uploader::sigv4::tests`

Expected: FAIL，签名接口尚不存在。

- [ ] **Step 3: 实现签名**

实现接口：

```rust
use chrono::{DateTime, Utc};
use file_uploader_sdk::error::UploadError;

pub struct Credentials { pub access_key_id: String, pub secret_access_key: String }

pub struct SigningInput<'a> {
    pub method: &'a str,
    pub canonical_uri: &'a str,
    pub canonical_query: &'a str,
    pub host: &'a str,
    pub content_type: Option<&'a str>,
    pub payload_hash: &'a str,
    pub timestamp: DateTime<Utc>,
    pub region: &'a str,
    pub service: &'a str,
}

pub struct SignedHeaders { pub amz_date: String, pub authorization: String }

pub fn encode_path(value: &str) -> String;
pub fn canonical_query(pairs: &[(&str, &str)]) -> String;
pub fn sign(credentials: &Credentials, input: &SigningInput<'_>)
    -> Result<SignedHeaders, UploadError>;
```

编码必须按 UTF-8 字节执行 RFC 3986：仅 `[A-Za-z0-9-._~]` 不转义；path 保留 `/`，query 不保留 `/`；百分号使用大写十六进制。canonical headers 固定按 `content-type`（若有）、`host`、`x-amz-content-sha256`、`x-amz-date` 字典序排列，signed headers 必须完全同序。

签名链：`kDate = HMAC("AWS4" + secret, YYYYMMDD)` → region → service → `aws4_request` → string-to-sign。任何 HMAC 构造错误映射为 `UploadError::InvalidFormat`。

- [ ] **Step 4: 运行测试并提交**

Run: `cargo test -p file_uploader_plugins common_uploader::sigv4::tests`

Expected: PASS，官方签名值完全一致。

```bash
git add file_uploader_plugins/src/upload/common_uploader/sigv4.rs
git commit -m "feat(upload): 实现 AWS SigV4 签名"
```

---

### Task 4: Multipart 纯函数

**Files:**
- Modify: `file_uploader_plugins/src/upload/common_uploader/multipart.rs`

**Interfaces:**
- Produces: `PartRange`、`part_ranges`、`parse_upload_id`、`complete_body`，R2 Multipart 流程使用。

- [ ] **Step 1: 写边界与 XML 失败测试**

```rust
#[test]
fn exact_part_size_is_one_part() {
    assert_eq!(part_ranges(64, 64).unwrap(), vec![PartRange { number: 1, offset: 0, size: 64 }]);
}

#[test]
fn remainder_creates_last_short_part() {
    assert_eq!(part_ranges(130, 64).unwrap(), vec![
        PartRange { number: 1, offset: 0, size: 64 },
        PartRange { number: 2, offset: 64, size: 64 },
        PartRange { number: 3, offset: 128, size: 2 },
    ]);
}

#[test]
fn rejects_more_than_ten_thousand_parts() {
    assert!(part_ranges(10001, 1).is_err());
}

#[test]
fn extracts_upload_id_and_decodes_entities() {
    assert_eq!(parse_upload_id("<InitiateMultipartUploadResult><UploadId>a&amp;b</UploadId></InitiateMultipartUploadResult>").unwrap(), "a&b");
}

#[test]
fn complete_body_escapes_etag() {
    assert_eq!(complete_body(&[(1, "\"a&b\"")]),
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;a&amp;b&quot;</ETag></Part></CompleteMultipartUpload>");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins common_uploader::multipart::tests`

Expected: FAIL。

- [ ] **Step 3: 实现纯函数**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange { pub number: u32, pub offset: u64, pub size: u64 }

pub fn part_ranges(total_size: u64, part_size: u64)
    -> Result<Vec<PartRange>, UploadError>;
pub fn parse_upload_id(xml: &str) -> Result<String, UploadError>;
pub fn complete_body(parts: &[(u32, String)]) -> String;
```

`part_size == 0`、零字节文件走 Multipart、分片数超过 10000 均返回 `InvalidFormat`。XML 解码仅支持本接口需要的五种实体 `&amp; &lt; &gt; &quot; &apos;`；缺失/空 `<UploadId>` 返回 `InvalidFormat`。

- [ ] **Step 4: 运行测试并提交**

Run: `cargo test -p file_uploader_plugins common_uploader::multipart::tests`

Expected: PASS。

```bash
git add file_uploader_plugins/src/upload/common_uploader/multipart.rs
git commit -m "feat(upload): 实现 Multipart 分片与 XML 工具"
```

---

### Task 5: Cloudflare R2 Provider

**Files:**
- Modify: `file_uploader_plugins/src/upload/common_uploader/r2.rs`
- Modify: `file_uploader_plugins/src/upload/common_uploader/provider.rs`

**Interfaces:**
- Consumes: Task 1 的 provider DTO、Task 3 的 SigV4、Task 4 的 Multipart 函数、`fs_util::open_read`。
- Produces: `R2Provider::new` 与完整 `StorageProvider` 实现。

- [ ] **Step 1: 将 HTTP 构造拆为可测纯函数并写失败测试**

为避免单元测试访问真实网络，先测试 endpoint、URL 与重试判定：

```rust
#[test]
fn object_url_encodes_key_and_public_url_uses_same_key() {
    let p = provider();
    assert_eq!(p.api_url("a b/中.png"),
        "https://acct.r2.cloudflarestorage.com/bucket/a%20b/%E4%B8%AD.png");
    assert_eq!(p.public_url("a b/中.png"),
        "https://cdn.example.com/a%20b/%E4%B8%AD.png");
}

#[test]
fn retries_only_transport_and_server_errors() {
    assert!(is_retryable_status(500));
    assert!(is_retryable_status(503));
    assert!(!is_retryable_status(403));
    assert!(!is_retryable_status(409));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins common_uploader::r2::tests`

Expected: FAIL。

- [ ] **Step 3: 实现配置与签名请求助手**

实现：

```rust
pub struct R2Provider {
    account_id: String,
    bucket: String,
    credentials: Credentials,
    public_base_url: String,
    overwrite: bool,
    timeout: Duration,
    retry_times: u32,
    client: reqwest::blocking::Client,
}

impl R2Provider {
    pub fn new(
        account_id: String,
        bucket: String,
        access_key_id: String,
        secret_access_key: String,
        public_base_url: String,
        overwrite: bool,
        timeout_secs: u64,
        retry_times: u32,
    ) -> Result<Self, UploadError>;

    fn api_url(&self, object_key: &str) -> String;
    fn public_url(&self, object_key: &str) -> String;
    fn send_signed(
        &self,
        method: Method,
        object_key: &str,
        query: &[(&str, &str)],
        payload_hash: &str,
        content_type: Option<&str>,
        body: Option<reqwest::blocking::Body>,
    ) -> Result<Response, UploadError>;
}
```

`send_signed` 每次尝试重新生成 `x-amz-date` 与 Authorization。错误消息必须包含方法、object key 和 status，但不得包含 access key、secret 或完整 Authorization。

- [ ] **Step 4: 实现 HEAD 与单次 PUT**

`overwrite=false` 时先 HEAD：200 表示存在并返回 `PluginParamInvalid("object key 已存在")`；404 继续；其他 2xx 视为存在；其他状态按统一错误处理。

单次 PUT 要求 `request.full_hash` 必须存在；用 `fs_util::open_read(request.work_dir, request.local_path)` 获得 reader，传给 `reqwest::blocking::Body::new(reader)`，不得 `read_to_end`。成功状态为任意 2xx，返回 `UploadedObject { url: public_url, object_key }`。

- [ ] **Step 5: 实现串行 Multipart**

严格执行：

1. `POST ?uploads=`（canonical query 的 value 为空），payload hash 使用空 body sha256。
2. 从 body 调 `parse_upload_id`。
3. 每片重新 `fs_util::open_read`，用 `Seek::seek(SeekFrom::Start(offset))` 定位，`Read::take(size).read_to_end(&mut Vec::with_capacity(size as usize))` 读取当前片；计算片 sha256 后 PUT `partNumber + uploadId`。
4. 每片成功后读取 `ETag` 响应头；缺失即失败。
5. `complete_body` 生成 XML，算 XML sha256，POST `uploadId`。
6. 任何 Create 之后的错误都尝试 DELETE `uploadId`；若 Abort 也失败，最终错误字符串追加 `uploadId` 与 `object_key`。

注意：`reqwest::blocking::Body` 不可 clone，因此每次重试前必须重新打开并读取当前片；不能在 `send_signed` 内对同一个 body 自动重试。将重试循环放在 `put_single`/`put_part` 外层，单次请求函数只发送一次。

- [ ] **Step 6: 完成 trait 实现**

```rust
impl StorageProvider for R2Provider {
    fn provider_id(&self) -> &'static str { "r2" }

    fn upload(&self, request: &UploadRequest<'_>) -> Result<UploadedObject, UploadError> {
        if !self.overwrite { self.ensure_absent(request.object_key)?; }
        match request.mode {
            UploadMode::Single => self.put_single(request)?,
            UploadMode::Multipart { part_size } => self.put_multipart(request, part_size)?,
        }
        Ok(UploadedObject {
            url: self.public_url(request.object_key),
            object_key: request.object_key.to_string(),
        })
    }
}
```

- [ ] **Step 7: 加真实 R2 ignored 测试**

新增两个 `#[ignore]` 测试，分别上传小文件和 65 MiB 文件。凭证环境变量固定命名：

- `R2_ACCOUNT_ID`
- `R2_BUCKET`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_PUBLIC_BASE_URL`

任一缺失则打印原因并 return。上传成功断言 URL 以 `R2_PUBLIC_BASE_URL` 开头。测试 object key 使用 `integration-tests/{gen_unique_name("bin")}`，避免覆盖用户文件。

- [ ] **Step 8: 运行离线测试并提交**

Run: `cargo test -p file_uploader_plugins common_uploader::r2::tests`

Expected: 非 ignored 测试 PASS，真实网络测试 ignored。

```bash
git add file_uploader_plugins/src/upload/common_uploader/{provider.rs,r2.rs}
git commit -m "feat(upload): 实现 Cloudflare R2 单次与分片上传"
```

---

### Task 6: 通用上传插件编排与资源

**Files:**
- Modify: `file_uploader_plugins/src/upload/common_uploader.rs`
- Create: `file_uploader_plugins/resources/upload/common_uploader/meta.json`
- Create: `file_uploader_plugins/resources/upload/common_uploader/config.json`
- Create: `file_uploader_plugins/resources/upload/common_uploader/README.md`

**Interfaces:**
- Consumes: Tasks 1–5 全部上传接口。
- Produces: 可注册的 `CommonUploader`，成功时把 file 改写成 NetworkPath。

- [ ] **Step 1: 写配置选路与输出契约失败测试**

在 `common_uploader.rs` 测试私有纯函数：

```rust
#[test]
fn default_options_use_date_uuid_and_64_mb_parts() {
    let ctx = ctx_with(json!({"group":"r2"}));
    let options = UploadOptions::from_ctx(&ctx).unwrap();
    assert_eq!(options.naming, NamingStrategy::DateUuid);
    assert!(options.multipart_enabled);
    assert_eq!(options.part_size, 64 * 1024 * 1024);
}

#[test]
fn size_equal_to_part_size_uses_single_put() {
    assert_eq!(select_mode(true, 64, 64), UploadMode::Single);
}

#[test]
fn size_above_part_size_uses_multipart() {
    assert_eq!(select_mode(true, 65, 64), UploadMode::Multipart { part_size: 64 });
}

#[test]
fn disabled_multipart_keeps_large_file_single() {
    assert_eq!(select_mode(false, 65, 64), UploadMode::Single);
}

#[test]
fn uploaded_file_is_network_path_and_preserves_metadata() {
    let output = build_success_output(file(), "/work/a.png", "r2", UploadedObject {
        url: "https://cdn.example.com/a.png".into(),
        object_key: "a.png".into(),
    });
    let out_file = output.file.unwrap();
    assert!(matches!(out_file.data_type, FileDataType::NetworkPath));
    assert_eq!(out_file.input_path, "https://cdn.example.com/a.png");
    assert_eq!(out_file.name, "a.png");
    assert_eq!(output.extra_info.unwrap().get("upload_local_path").unwrap(), "/work/a.png");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins upload::common_uploader::tests`

Expected: FAIL。

- [ ] **Step 3: 实现配置解析、validate_params 与 execute**

实现默认常量：

```rust
const DEFAULT_TIMEOUT_SECS: i64 = 30;
const DEFAULT_RETRY_TIMES: i64 = 2;
const DEFAULT_PART_SIZE: u64 = 64 * 1024 * 1024;
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;
```

`UploadOptions::from_ctx` 使用现有 `config_util::{get_bool,get_group,get_i64,get_size,get_str}`；default_value 不会运行时合并，代码必须 `unwrap_or`。

`validate_params` 验证：group 必须为 r2；`public_base_url` 以 http(s) 开头且等于 trim 后值；prefix 不以 `/` 开头且不含 `..`；naming 可解析；part size 在范围内。文件大小相关的 10000 片限制在 `execute` 校验：`ceil(size / part_size) <= 10000`。

`run` 流程：读取 options → 取 file/work_dir → select_mode → 仅 Single 或 Hash 命名调用 `sha256_file` → build key → 创建 R2Provider → provider.upload → build_success_output。`execute` 只把 `Result` 映射为 success/failed，不记录配置。

- [ ] **Step 4: 创建 meta.json 与 config.json**

`config.json` 必须完整表达 spec 的字段，尤其：

```json
{
  "access": {"fs_read": true, "fs_write": false, "network": true},
  "common": [
    {"key":"timeout_secs","title":"请求超时（秒）","description":"单次 HTTP 请求的超时秒数","config_type":"Default","default_value":30,"required":false,"form":{"type":"number","min":1,"max":3600,"step":1,"integer":true}},
    {"key":"retry_times","title":"失败重试次数","description":"单次上传或每个分片独立重试的次数","config_type":"Default","default_value":2,"required":false,"form":{"type":"number","min":0,"max":5,"step":1,"integer":true}},
    {"key":"naming","title":"对象命名方式","description":"上传后的对象键生成方式","config_type":"Default","default_value":"date_uuid","required":false,"form":{"type":"select","multiple":false,"allow_custom":false,"options":[{"label":"原文件名","value":"origin"},{"label":"唯一名称","value":"uuid"},{"label":"内容哈希","value":"hash"},{"label":"日期目录 + 唯一名称","value":"date_uuid"}]}},
    {"key":"key_prefix","title":"对象键前缀","description":"例如 uploads/，不得以 / 开头或包含 ..","config_type":"Default","default_value":"","required":false,"form":{"type":"text","secret":false,"max_len":256}},
    {"key":"multipart_enabled","title":"启用分片上传","description":"开启后，大于分片大小的文件使用串行分片上传","config_type":"Default","default_value":true,"required":false,"form":{"type":"switch"}},
    {"key":"multipart_part_size","title":"分片大小","description":"默认 64MB，范围 5MB 至 5GB","config_type":"Default","default_value":"64MB","required":false,"form":{"type":"text","secret":false,"min_len":2,"max_len":16}}
  ],
  "groups": [{
    "group":"r2","title":"Cloudflare R2","description":"上传到 Cloudflare R2 对象存储","params":[
      {"key":"account_id","title":"Account ID","description":"Cloudflare 账户 ID","config_type":"Default","default_value":"","required":true,"form":{"type":"text","secret":false,"min_len":1,"max_len":128}},
      {"key":"bucket","title":"Bucket","description":"R2 存储桶名称","config_type":"Default","default_value":"","required":true,"form":{"type":"text","secret":false,"min_len":1,"max_len":255}},
      {"key":"access_key_id","title":"Access Key ID","description":"R2 API Token 的 Access Key ID","config_type":"Default","default_value":"","required":true,"form":{"type":"text","secret":false,"min_len":1,"max_len":256}},
      {"key":"secret_access_key","title":"Secret Access Key","description":"R2 API Token 的密钥","config_type":"Default","default_value":"","required":true,"form":{"type":"text","secret":true,"min_len":1,"max_len":256}},
      {"key":"public_base_url","title":"公开访问域名","description":"例如 https://cdn.example.com","config_type":"Default","default_value":"","required":true,"form":{"type":"text","secret":false,"min_len":1,"max_len":2048}},
      {"key":"overwrite","title":"允许覆盖同名对象","description":"关闭时上传前检查对象是否存在","config_type":"Default","default_value":true,"required":false,"form":{"type":"switch"}}
    ]
  }]
}
```

- [ ] **Step 5: 写面向配置者的 README**

章节固定为：能做什么、什么时候用它、配置方法、参数一览、上传结果、常见问题。必须明确：

- 先配置并运行 `default_input_handler`，确保文件已进入 work_dir。
- R2 自定义域或 r2.dev 域必须作为 `public_base_url`。
- 默认小于等于 64MB 单次上传，大于 64MB 串行分片；分片不是为了降低内存。
- `uuid`/`date_uuid` 的唯一名不是 RFC 4122 UUID。
- Abort 失败消息中的 uploadId 用于手工清理。

- [ ] **Step 6: 运行插件与 schema 测试**

Run: `cargo test -p file_uploader_plugins upload::common_uploader`

Expected: PASS。

Run: `cargo test -p file_uploader_sdk validate_util`

Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add file_uploader_plugins/src/upload/common_uploader.rs file_uploader_plugins/resources/upload/common_uploader
git commit -m "feat(upload): 接入通用上传插件配置与输出契约"
```

---

### Task 7: 输出渲染纯函数

**Files:**
- Modify: `file_uploader_plugins/src/output/common_output/render.rs`

**Interfaces:**
- Produces: `OutputFormat::parse` 与 `render`，CommonOutput 使用。

- [ ] **Step 1: 写三种格式、转义与模板失败测试**

```rust
#[test]
fn markdown_uses_image_syntax_and_escapes_name() {
    let values = Values { name: "a[1].png", url: "https://x/a (1).png", size: 12, file_type: "image/png" };
    assert_eq!(render(OutputFormat::Markdown, &values, None).unwrap(),
               "![a\\[1\\].png](https://x/a%20%281%29.png)");
}

#[test]
fn markdown_uses_link_for_non_image() {
    let values = Values { name: "a.pdf", url: "https://x/a.pdf", size: 12, file_type: "application/pdf" };
    assert_eq!(render(OutputFormat::Markdown, &values, None).unwrap(),
               "[a.pdf](https://x/a.pdf)");
}

#[test]
fn html_escapes_name_and_url() {
    let values = Values { name: "a&\".png", url: "https://x/?a=1&b=2", size: 12, file_type: "image/png" };
    assert_eq!(render(OutputFormat::Html, &values, None).unwrap(),
               "<img src=\"https://x/?a=1&amp;b=2\" alt=\"a&amp;&quot;.png\">");
}

#[test]
fn link_returns_url() {
    let values = Values { name: "a", url: "https://x/a", size: 12, file_type: "text/plain" };
    assert_eq!(render(OutputFormat::Link, &values, None).unwrap(), "https://x/a");
}

#[test]
fn template_overrides_format_and_replaces_all_values() {
    let values = Values { name: "a.png", url: "https://x/a.png", size: 12, file_type: "image/png" };
    assert_eq!(render(OutputFormat::Html, &values, Some("{name}|{url}|{size}|{type}")).unwrap(),
        "a.png|https://x/a.png|12|image/png");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins output::common_output::render::tests`

Expected: FAIL。

- [ ] **Step 3: 实现渲染接口**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat { Markdown, Html, Link }

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String>;
    pub fn as_str(self) -> &'static str;
}

pub struct Values<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub size: usize,
    pub file_type: &'a str,
}

pub fn render(format: OutputFormat, values: &Values<'_>, template: Option<&str>)
    -> Result<String, String>;
```

模板非空时优先，按 `{url}`、`{name}`、`{size}`、`{type}` 做普通字符串替换且不转义。内置 HTML 转义五字符；Markdown 名称转义 `\\ [ ] ( )`，URL 将空格、`(`、`)` 编码为 `%20`、`%28`、`%29`。

- [ ] **Step 4: 运行测试并提交**

Run: `cargo test -p file_uploader_plugins output::common_output::render::tests`

Expected: PASS。

```bash
git add file_uploader_plugins/src/output/common_output/render.rs
git commit -m "feat(output): 实现链接格式化与安全转义"
```

---

### Task 8: 通用输出插件与资源

**Files:**
- Modify: `file_uploader_plugins/src/output/common_output.rs`
- Create: `file_uploader_plugins/resources/output/common_output/meta.json`
- Create: `file_uploader_plugins/resources/output/common_output/config.json`
- Create: `file_uploader_plugins/resources/output/common_output/README.md`

**Interfaces:**
- Consumes: Task 7 `render`。
- Produces: 可注册的 `CommonOutput`。

- [ ] **Step 1: 写输入契约与输出失败测试**

构造 `UploadInputCtx` 后覆盖：

```rust
#[test]
fn fails_without_file() {
    let out = run(None, json!({}));
    assert!(matches!(out.result, OutputResultType::Failed));
    assert!(out.message.contains("未获得可访问 URL"));
}

#[test]
fn fails_for_local_file() {
    let out = run(Some(file(FileDataType::FilePath, "/tmp/a.png")), json!({}));
    assert!(matches!(out.result, OutputResultType::Failed));
}

#[test]
fn defaults_to_markdown_and_preserves_file() {
    let out = run(Some(file(FileDataType::NetworkPath, "https://x/a.png")), json!({}));
    assert_eq!(out.message, "![a.png](https://x/a.png)");
    assert_eq!(out.file.unwrap().input_path, "https://x/a.png");
    let info = out.extra_info.unwrap();
    assert_eq!(info.get("output_format").unwrap(), "markdown");
    assert_eq!(info.get("output").unwrap(), "![a.png](https://x/a.png)");
}

#[test]
fn template_sets_template_format() {
    let out = run(Some(file(FileDataType::NetworkPath, "https://x/a.png")),
                  json!({"format":"html", "template":"{url}"}));
    assert_eq!(out.message, "https://x/a.png");
    assert_eq!(out.extra_info.unwrap().get("output_format").unwrap(), "template");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins output::common_output::tests`

Expected: FAIL。

- [ ] **Step 3: 实现 CommonOutput**

`execute` 验证 file 存在、`data_type` 为 `NetworkPath`、URL 以 `http://` 或 `https://` 开头；读取 format（默认 markdown）和 template；调用 render；把原 file 原样回传，并合并原 `extra_info` 后写入 `output`、`output_format`。错误统一前缀 `common_output:`。

`validate_params` 只验证 format 可解析；template 的长度由 schema 声明式校验承担。

- [ ] **Step 4: 创建资源文件**

`meta.json`：name=`common_output`、phase=`Output`、version=`0.0.1`。

`config.json`：

```json
{
  "access":{"fs_read":false,"fs_write":false,"network":false},
  "common":[
    {"key":"format","title":"输出格式","description":"选择 markdown、HTML 或纯超链接","config_type":"Default","default_value":"markdown","required":false,"form":{"type":"select","multiple":false,"allow_custom":false,"options":[{"label":"Markdown","value":"markdown"},{"label":"HTML","value":"html"},{"label":"超链接","value":"link"}]}},
    {"key":"template","title":"自定义模板","description":"非空时覆盖输出格式，可使用 {url} {name} {size} {type}","config_type":"Default","default_value":"","required":false,"form":{"type":"text","secret":false,"max_len":512}}
  ],
  "groups":[]
}
```

README 面向使用者，写：能做什么、什么时候用它、配置方法、格式示例、模板占位符、常见问题；明确只接受上游产出的 `NetworkPath`，模板内容不自动转义。

- [ ] **Step 5: 运行测试并提交**

Run: `cargo test -p file_uploader_plugins output::common_output`

Expected: PASS。

```bash
git add file_uploader_plugins/src/output file_uploader_plugins/resources/output/common_output
git commit -m "feat(output): 实现通用输出插件"
```

---

### Task 9: 规范同步与全量验证

**Files:**
- Modify: `docs/references/plugin-specification.md`

**Interfaces:**
- Consumes: 两个插件的最终行为。
- Produces: 与代码一致的长期规范。

- [ ] **Step 1: 更新规范**

在资源目录约定中增加：

```text
Output 阶段的进程内插件使用 resources/output/<plugin_name>/。
```

在上传阶段与数据流章节增加：Upload 阶段成功后应优先通过 `UploadFileData` 原生表达远程产物：`data_type = NetworkPath`、`input_path = 可访问 URL`；`extra_info` 仅承载 provider/object_key/local_path 等辅助信息。Output 插件消费该 file，不绑定某个上传插件私有 key。

在 README 规范示例中说明 secret 参数不得出现在日志与错误消息。

- [ ] **Step 2: 格式化并运行所有测试**

Run: `cargo fmt --all -- --check`

Expected: PASS；如失败，先运行 `cargo fmt --all`，再重复 check。

Run: `cargo test -p file_uploader_plugins`

Expected: 全部非 ignored 测试 PASS。

Run: `cargo test --workspace`

Expected: workspace 全部非 ignored 测试 PASS。

Run: `cargo build --workspace`

Expected: PASS。

- [ ] **Step 3: 核对资源复制**

Run: `cargo build -p file_uploader_plugins && test -f target/debug/resources/upload/common_uploader/meta.json && test -f target/debug/resources/output/common_output/meta.json`

Expected: exit code 0。

- [ ] **Step 4: 搜索敏感日志与未实现占位**

Run: `rg 'secret_access_key|serde_json::to_string\(ctx\)|not configured|todo!|unimplemented!' file_uploader_plugins/src/upload/common_uploader file_uploader_plugins/src/output/common_output`

Expected: `secret_access_key` 只出现在配置读取/结构字段中；无 ctx 序列化、`not configured`、`todo!` 或 `unimplemented!`。

- [ ] **Step 5: 提交文档与格式化收尾**

```bash
git add docs/references/plugin-specification.md file_uploader_plugins
git commit -m "docs(plugins): 补充上传结果与输出目录规范"
```

- [ ] **Step 6: 最终工作区检查**

Run: `git status --short`

Expected: 空输出；不得自动 push，除非用户明确要求。
