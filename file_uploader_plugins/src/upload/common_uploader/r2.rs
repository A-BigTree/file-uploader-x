use super::multipart::{complete_body, parse_upload_id, part_ranges, PartRange};
use super::provider::{StorageProvider, UploadMode, UploadRequest, UploadedObject};
use super::sigv4::{canonical_query, encode_path, sign, Credentials, SigningInput};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::utils::fs_util;
use reqwest::blocking::{Body, Client, Response};
use reqwest::{Method, StatusCode};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const R2_REGION: &str = "auto";
const R2_SERVICE: &str = "s3";

pub struct R2Provider {
    account_id: String,
    bucket: String,
    credentials: Credentials,
    public_base_url: String,
    overwrite: bool,
    #[allow(dead_code)]
    timeout: Duration,
    retry_times: u32,
    client: Client,
}

impl R2Provider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account_id: String,
        bucket: String,
        access_key_id: String,
        secret_access_key: String,
        public_base_url: String,
        overwrite: bool,
        timeout_secs: u64,
        retry_times: u32,
    ) -> Result<Self, UploadError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| UploadError::InvalidFormat(format!("reqwest client build failed: {e}")))?;
        Ok(Self {
            account_id,
            bucket,
            credentials: Credentials {
                access_key_id,
                secret_access_key,
            },
            public_base_url,
            overwrite,
            timeout: Duration::from_secs(timeout_secs),
            retry_times,
            client,
        })
    }

    fn host(&self) -> String {
        format!("{}.r2.cloudflarestorage.com", self.account_id)
    }

    fn api_url(&self, object_key: &str) -> String {
        format!("https://{}/{}{}", self.host(), self.bucket, encode_path(object_key))
    }

    fn public_url(&self, object_key: &str) -> String {
        format!(
            "{}{}",
            self.public_base_url.trim_end_matches('/'),
            encode_path(object_key)
        )
    }

    fn send_signed(
        &self,
        method: Method,
        object_key: &str,
        query: &[(&str, &str)],
        payload_hash: &str,
        content_type: Option<&str>,
        body: Option<Body>,
    ) -> Result<Response, UploadError> {
        let host = self.host();
        let canonical_uri = encode_path(object_key);
        let cquery = canonical_query(query);
        let signing_input = SigningInput {
            method: method.as_str(),
            canonical_uri: &canonical_uri,
            canonical_query: &cquery,
            host: &host,
            content_type,
            payload_hash,
            timestamp: chrono::Utc::now(),
            region: R2_REGION,
            service: R2_SERVICE,
        };
        let signed = sign(&self.credentials, &signing_input)?;

        let full_url = if cquery.is_empty() {
            self.api_url(object_key)
        } else {
            format!("{}?{}", self.api_url(object_key), cquery)
        };

        let mut req = self.client.request(method.clone(), &full_url);
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        req = req
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", &signed.amz_date)
            .header("authorization", &signed.authorization);
        if let Some(b) = body {
            req = req.body(b);
        }
        req.send().map_err(|e| {
            UploadError::InvalidFormat(format!("{} {} transport error: {e}", method.as_str(), object_key))
        })
    }

    /// 单次签名发送 + 针对传输错误与 5xx 的串行重试。
    /// 每次重试都调用 `body_fn` 重新生成 payload hash 与 body（Body 不可 clone）。
    /// 返回最后一次响应（任意状态码）；传输错误重试耗尽后返回最后一次错误。
    fn request_with_retry(
        &self,
        method: Method,
        object_key: &str,
        query: &[(&str, &str)],
        content_type: Option<&str>,
        mut body_fn: impl FnMut() -> Result<(String, Option<Body>), UploadError>,
    ) -> Result<Response, UploadError> {
        let attempts = self.retry_times.saturating_add(1);
        let mut last: Option<Result<Response, UploadError>> = None;
        for _ in 0..attempts {
            let (payload_hash, body) = body_fn()?;
            match self.send_signed(method.clone(), object_key, query, &payload_hash, content_type, body) {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if !is_retryable_status(status) {
                        return Ok(resp);
                    }
                    last = Some(Err(UploadError::InvalidFormat(format!(
                        "{} {} retryable status: HTTP {status}",
                        method.as_str(),
                        object_key
                    ))));
                }
                Err(e) => {
                    last = Some(Err(e));
                }
            }
        }
        last.unwrap_or_else(|| Err(UploadError::InvalidFormat("no retry attempts".into())))
    }

    fn ensure_absent(&self, object_key: &str) -> Result<(), UploadError> {
        let resp = self.request_with_retry(Method::HEAD, object_key, &[], None, || {
            Ok((EMPTY_SHA256.to_string(), None))
        })?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            Ok(())
        } else if status.is_success() {
            Err(UploadError::PluginParamInvalid("object key 已存在".into()))
        } else {
            Err(UploadError::InvalidFormat(format!(
                "HEAD {object_key} unexpected status: HTTP {status}"
            )))
        }
    }

    fn put_single(&self, request: &UploadRequest<'_>) -> Result<(), UploadError> {
        let full_hash = request.full_hash.ok_or_else(|| {
            UploadError::PluginParamInvalid("single put requires full_hash".into())
        })?;
        let work_dir = request.work_dir.to_string();
        let local_path = request.local_path.to_string();
        let resp = self.request_with_retry(
            Method::PUT,
            request.object_key,
            &[],
            Some(request.content_type),
            || {
                let reader = fs_util::open_read(&work_dir, &local_path)?;
                Ok((full_hash.to_string(), Some(Body::new(reader))))
            },
        )?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(UploadError::InvalidFormat(format!(
                "PUT {} failed: HTTP {status}",
                request.object_key
            )))
        }
    }

    fn initiate(&self, object_key: &str) -> Result<String, UploadError> {
        let resp = self.request_with_retry(
            Method::POST,
            object_key,
            &[("uploads", "")],
            None,
            || Ok((EMPTY_SHA256.to_string(), None)),
        )?;
        let status = resp.status();
        if !status.is_success() {
            return Err(UploadError::InvalidFormat(format!(
                "initiate multipart {object_key} failed: HTTP {status}"
            )));
        }
        let body = resp
            .text()
            .map_err(|e| UploadError::InvalidFormat(format!("read initiate body: {e}")))?;
        parse_upload_id(&body)
    }

    fn put_part(
        &self,
        request: &UploadRequest<'_>,
        upload_id: &str,
        range: PartRange,
    ) -> Result<String, UploadError> {
        let work_dir = request.work_dir.to_string();
        let local_path = request.local_path.to_string();
        let object_key = request.object_key.to_string();
        let part_no = range.number.to_string();
        let upload_id = upload_id.to_string();
        let content_type = request.content_type;
        let query = [("partNumber", part_no.as_str()), ("uploadId", upload_id.as_str())];
        let offset = range.offset;
        let size = range.size;
        let number = range.number;
        let resp = self.request_with_retry(
            Method::PUT,
            &object_key,
            &query,
            Some(content_type),
            || {
                let mut reader = fs_util::open_read(&work_dir, &local_path)?;
                reader.seek(SeekFrom::Start(offset))?;
                let mut buf = Vec::with_capacity(size as usize);
                reader.take(size).read_to_end(&mut buf)?;
                let hash = sha256_hex(&buf);
                Ok((hash, Some(Body::from(buf))))
            },
        )?;
        let status = resp.status();
        if !status.is_success() {
            return Err(UploadError::InvalidFormat(format!(
                "PUT part {object_key} #{number} failed: HTTP {status}"
            )));
        }
        let etag = resp
            .headers()
            .get("etag")
            .ok_or_else(|| UploadError::InvalidFormat(format!("missing ETag for part {number}")))?
            .to_str()
            .map_err(|e| UploadError::InvalidFormat(format!("invalid ETag header: {e}")))?
            .to_string();
        Ok(etag)
    }

    fn complete(
        &self,
        object_key: &str,
        upload_id: &str,
        etags: &[(u32, String)],
    ) -> Result<(), UploadError> {
        let xml = complete_body(etags);
        let resp = self.request_with_retry(
            Method::POST,
            object_key,
            &[("uploadId", upload_id)],
            None,
            || {
                let hash = sha256_hex(xml.as_bytes());
                Ok((hash, Some(Body::from(xml.as_bytes().to_vec()))))
            },
        )?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(UploadError::InvalidFormat(format!(
                "complete multipart {object_key} failed: HTTP {status}"
            )))
        }
    }

    fn abort_quiet(&self, object_key: &str, upload_id: &str) -> Result<(), UploadError> {
        let resp = self.request_with_retry(
            Method::DELETE,
            object_key,
            &[("uploadId", upload_id)],
            None,
            || Ok((EMPTY_SHA256.to_string(), None)),
        )?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(UploadError::InvalidFormat(format!(
                "abort multipart {object_key} failed: HTTP {status}"
            )))
        }
    }

    fn put_multipart(
        &self,
        request: &UploadRequest<'_>,
        part_size: u64,
    ) -> Result<(), UploadError> {
        let ranges = part_ranges(request.size, part_size)?;
        let upload_id = self.initiate(request.object_key)?;
        let result = self.upload_parts_and_complete(request, &upload_id, &ranges);
        if let Err(e) = result {
            let abort_msg = match self.abort_quiet(request.object_key, &upload_id) {
                Ok(()) => String::new(),
                Err(ae) => format!("; abort failed: {ae}"),
            };
            return Err(UploadError::InvalidFormat(format!(
                "{e}; uploadId={upload_id} object_key={}{}",
                request.object_key, abort_msg
            )));
        }
        Ok(())
    }

    fn upload_parts_and_complete(
        &self,
        request: &UploadRequest<'_>,
        upload_id: &str,
        ranges: &[PartRange],
    ) -> Result<(), UploadError> {
        let mut etags: Vec<(u32, String)> = Vec::with_capacity(ranges.len());
        for range in ranges {
            let etag = self.put_part(request, upload_id, *range)?;
            etags.push((range.number, etag));
        }
        self.complete(request.object_key, upload_id, &etags)
    }
}

impl StorageProvider for R2Provider {
    fn provider_id(&self) -> &'static str {
        "r2"
    }

    fn upload(&self, request: &UploadRequest<'_>) -> Result<UploadedObject, UploadError> {
        if !self.overwrite {
            self.ensure_absent(request.object_key)?;
        }
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

fn is_retryable_status(status: u16) -> bool {
    status >= 500
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> R2Provider {
        R2Provider::new(
            "acct".into(),
            "bucket".into(),
            "AKIAIOSFODNN7EXAMPLE".into(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            "https://cdn.example.com".into(),
            true,
            30,
            2,
        )
        .unwrap()
    }

    #[test]
    fn object_url_encodes_key_and_public_url_uses_same_key() {
        let p = provider();
        assert_eq!(
            p.api_url("a b/中.png"),
            "https://acct.r2.cloudflarestorage.com/bucket/a%20b/%E4%B8%AD.png"
        );
        assert_eq!(
            p.public_url("a b/中.png"),
            "https://cdn.example.com/a%20b/%E4%B8%AD.png"
        );
    }

    #[test]
    fn retries_only_transport_and_server_errors() {
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(409));
    }

    fn env(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|s| !s.is_empty())
    }

    fn real_provider() -> Option<R2Provider> {
        let account_id = env("R2_ACCOUNT_ID")?;
        let bucket = env("R2_BUCKET")?;
        let access_key_id = env("R2_ACCESS_KEY_ID")?;
        let secret_access_key = env("R2_SECRET_ACCESS_KEY")?;
        let public_base_url = env("R2_PUBLIC_BASE_URL")?;
        R2Provider::new(
            account_id,
            bucket,
            access_key_id,
            secret_access_key,
            public_base_url.clone(),
            true,
            60,
            2,
        )
        .ok()
    }

    fn write_temp_file(work_dir: &str, ext: &str, data: &[u8]) -> String {
        let (name, _path) = fs_util::write(work_dir, ext, &data[..]).unwrap();
        name
    }

    #[test]
    #[ignore]
    fn integration_upload_small_file_single_put() {
        let provider = match real_provider() {
            Some(p) => p,
            None => {
                eprintln!("skipped: R2_* env vars not set");
                return;
            }
        };
        let public_base = env("R2_PUBLIC_BASE_URL").unwrap();
        let tmp = std::env::temp_dir().join(format!("r2it_small_{}", fs_util::gen_unique_name("")));
        let work_dir = fs_util::create_work_dir(&tmp, "wd").unwrap();
        let work_dir_str = work_dir.to_str().unwrap().to_string();
        let data = b"hello-r2-integration";
        let name = write_temp_file(&work_dir_str, "bin", data);
        let object_key = format!("integration-tests/{}", fs_util::gen_unique_name("bin"));
        let full_hash = super::super::naming::sha256_file(&work_dir_str, &name).unwrap();
        let request = UploadRequest {
            work_dir: &work_dir_str,
            local_path: &name,
            object_key: &object_key,
            content_type: "application/octet-stream",
            size: data.len() as u64,
            full_hash: Some(&full_hash),
            mode: UploadMode::Single,
        };
        let uploaded = StorageProvider::upload(&provider, &request).unwrap();
        assert!(uploaded.url.starts_with(&public_base), "url={}", uploaded.url);
        assert_eq!(uploaded.object_key, object_key);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    #[ignore]
    fn integration_upload_large_file_multipart() {
        let provider = match real_provider() {
            Some(p) => p,
            None => {
                eprintln!("skipped: R2_* env vars not set");
                return;
            }
        };
        let public_base = env("R2_PUBLIC_BASE_URL").unwrap();
        let tmp = std::env::temp_dir().join(format!("r2it_big_{}", fs_util::gen_unique_name("")));
        let work_dir = fs_util::create_work_dir(&tmp, "wd").unwrap();
        let work_dir_str = work_dir.to_str().unwrap().to_string();
        let size: usize = 65 * 1024 * 1024;
        let data = vec![7u8; size];
        let name = write_temp_file(&work_dir_str, "bin", &data);
        let object_key = format!("integration-tests/{}", fs_util::gen_unique_name("bin"));
        let request = UploadRequest {
            work_dir: &work_dir_str,
            local_path: &name,
            object_key: &object_key,
            content_type: "application/octet-stream",
            size: size as u64,
            full_hash: None,
            mode: UploadMode::Multipart {
                part_size: 64 * 1024 * 1024,
            },
        };
        let uploaded = StorageProvider::upload(&provider, &request).unwrap();
        assert!(uploaded.url.starts_with(&public_base), "url={}", uploaded.url);
        assert_eq!(uploaded.object_key, object_key);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
