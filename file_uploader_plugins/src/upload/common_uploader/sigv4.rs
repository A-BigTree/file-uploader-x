use chrono::{DateTime, Utc};
use file_uploader_sdk::error::UploadError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}

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

pub struct SignedHeaders {
    pub amz_date: String,
    pub authorization: String,
}

pub fn encode_path(value: &str) -> String {
    let with_slash = if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{value}")
    };
    encode_with(with_slash.as_bytes(), true)
}

pub fn canonical_query(pairs: &[(&str, &str)]) -> String {
    let mut sorted: Vec<(&str, &str)> = pairs.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    sorted
        .into_iter()
        .map(|(k, v)| format!("{}={}", encode_value(k), encode_value(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_value(s: &str) -> String {
    encode_with(s.as_bytes(), false)
}

fn encode_with(bytes: &[u8], keep_slash: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let unreserved = matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~');
        if unreserved || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, UploadError> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|_| UploadError::InvalidFormat("invalid hmac key".into()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn derive_signing_key(
    secret: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, UploadError> {
    let k_secret = format!("AWS4{secret}");
    let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    let k_signing = hmac_sha256(&k_service, b"aws4_request")?;
    Ok(k_signing)
}

pub fn sign(credentials: &Credentials, input: &SigningInput<'_>) -> Result<SignedHeaders, UploadError> {
    let amz_date = input.timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = input.timestamp.format("%Y%m%d").to_string();

    let mut headers: Vec<(&str, String)> = Vec::new();
    if let Some(ct) = input.content_type {
        headers.push(("content-type", ct.to_string()));
    }
    headers.push(("host", input.host.to_string()));
    headers.push(("x-amz-content-sha256", input.payload_hash.to_string()));
    headers.push(("x-amz-date", amz_date.clone()));
    headers.sort_by(|a, b| a.0.cmp(b.0));

    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect();
    let signed_headers: String = headers
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        input.method,
        input.canonical_uri,
        input.canonical_query,
        canonical_headers,
        signed_headers,
        input.payload_hash
    );

    let credential_scope = format!("{date_stamp}/{}/{}/aws4_request", input.region, input.service);
    let hashed = sha256_hex(canonical_request.as_bytes());
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{hashed}"
    );

    let signing_key = derive_signing_key(&credentials.secret_access_key, &date_stamp, input.region, input.service)?;
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    );

    Ok(SignedHeaders { amz_date, authorization })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono::Utc;

    #[test]
    fn hmac_chain_matches_aws_iam_official_vector() {
        // 用 AWS 官方 IAM ListUsers 向量（headers: content-type;host;x-amz-date，
        // 不含 x-amz-content-sha256）手动构造 canonical request，验证 HMAC 派生链、
        // string-to-sign 组装与 sha256 计算均与官方一致。
        let credentials = Credentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
        };
        let ts = Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap();
        let amz_date = ts.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = ts.format("%Y%m%d").to_string();

        let canonical_headers = "content-type:application/x-www-form-urlencoded; charset=utf-8\n\
            host:iam.amazonaws.com\n\
            x-amz-date:20150830T123600Z\n";
        let canonical_request = format!(
            "GET\n/\nAction=ListUsers&Version=2010-05-08\n{canonical_headers}\n\
            content-type;host;x-amz-date\n\
            e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let credential_scope = format!("{date_stamp}/us-east-1/iam/aws4_request");
        let hashed = sha256_hex(canonical_request.as_bytes());
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{hashed}");
        let signing_key =
            derive_signing_key(&credentials.secret_access_key, &date_stamp, "us-east-1", "iam")
                .unwrap();
        let signature =
            hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()).unwrap());
        assert_eq!(
            signature,
            "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }

    #[test]
    fn sign_produces_r2_style_authorization() {
        // R2/S3 风格：固定包含 x-amz-content-sha256 与 x-amz-date。
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
        assert!(signed
            .authorization
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request"));
        assert!(signed.authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
        assert!(signed.authorization.ends_with(
            "Signature=65f031d93b4631aedf16a8f7f830cdc8ce2bc5276c307b5a2cc2143d4b68e323"
        ));
    }

    #[test]
    fn uri_encoding_rules() {
        assert_eq!(encode_path("bucket/a b/中.png"), "/bucket/a%20b/%E4%B8%AD.png");
        assert_eq!(
            canonical_query(&[("uploadId", "a+b/="), ("partNumber", "2")]),
            "partNumber=2&uploadId=a%2Bb%2F%3D"
        );
    }
}
