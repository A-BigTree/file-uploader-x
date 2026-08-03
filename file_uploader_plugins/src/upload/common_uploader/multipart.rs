use file_uploader_sdk::error::UploadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange {
    pub number: u32,
    pub offset: u64,
    pub size: u64,
}

pub fn part_ranges(total_size: u64, part_size: u64) -> Result<Vec<PartRange>, UploadError> {
    if part_size == 0 {
        return Err(UploadError::InvalidFormat(
            "multipart part_size is 0".into(),
        ));
    }
    if total_size == 0 {
        return Err(UploadError::InvalidFormat(
            "multipart upload of empty file is not allowed".into(),
        ));
    }
    let count = total_size.div_ceil(part_size);
    if count > 10000 {
        return Err(UploadError::InvalidFormat(format!(
            "multipart parts {count} exceed 10000"
        )));
    }
    let mut ranges = Vec::with_capacity(count as usize);
    let mut offset = 0u64;
    let mut number = 1u32;
    while offset < total_size {
        let size = part_size.min(total_size - offset);
        ranges.push(PartRange {
            number,
            offset,
            size,
        });
        offset += size;
        number += 1;
    }
    Ok(ranges)
}

fn extract_tag(xml: &str, tag: &str) -> Result<String, UploadError> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml
        .find(&open)
        .ok_or_else(|| UploadError::InvalidFormat(format!("missing <{tag}>")))?;
    let after = start + open.len();
    let end = xml[after..]
        .find(&close)
        .ok_or_else(|| UploadError::InvalidFormat(format!("missing </{tag}>")))?;
    Ok(xml[after..after + end].to_string())
}

fn decode_entities(s: &str) -> String {
    let entities = [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&apos;", "'")];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if let Some((entity, replacement)) = entities
            .into_iter()
            .find(|(e, _)| after.starts_with(e))
        {
            out.push_str(replacement);
            rest = &after[entity.len()..];
        } else {
            out.push('&');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

pub fn parse_upload_id(xml: &str) -> Result<String, UploadError> {
    let raw = extract_tag(xml, "UploadId")?;
    let id = decode_entities(&raw);
    if id.is_empty() {
        return Err(UploadError::InvalidFormat("empty UploadId".into()));
    }
    Ok(id)
}

fn encode_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn complete_body(parts: &[(u32, String)]) -> String {
    let mut out = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        out.push_str("<Part><PartNumber>");
        out.push_str(&number.to_string());
        out.push_str("</PartNumber><ETag>");
        out.push_str(&encode_xml_text(etag));
        out.push_str("</ETag></Part>");
    }
    out.push_str("</CompleteMultipartUpload>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_part_size_is_one_part() {
        assert_eq!(
            part_ranges(64, 64).unwrap(),
            vec![PartRange {
                number: 1,
                offset: 0,
                size: 64
            }]
        );
    }

    #[test]
    fn remainder_creates_last_short_part() {
        assert_eq!(
            part_ranges(130, 64).unwrap(),
            vec![
                PartRange {
                    number: 1,
                    offset: 0,
                    size: 64
                },
                PartRange {
                    number: 2,
                    offset: 64,
                    size: 64
                },
                PartRange {
                    number: 3,
                    offset: 128,
                    size: 2
                },
            ]
        );
    }

    #[test]
    fn rejects_more_than_ten_thousand_parts() {
        assert!(part_ranges(10001, 1).is_err());
    }

    #[test]
    fn rejects_zero_part_size_and_empty_file() {
        assert!(part_ranges(10, 0).is_err());
        assert!(part_ranges(0, 64).is_err());
    }

    #[test]
    fn extracts_upload_id_and_decodes_entities() {
        assert_eq!(
            parse_upload_id(
                "<InitiateMultipartUploadResult><UploadId>a&amp;b</UploadId></InitiateMultipartUploadResult>"
            )
            .unwrap(),
            "a&b"
        );
    }

    #[test]
    fn missing_or_empty_upload_id_is_error() {
        assert!(parse_upload_id("<x></x>").is_err());
        assert!(parse_upload_id("<InitiateMultipartUploadResult><UploadId></UploadId></InitiateMultipartUploadResult>").is_err());
    }

    #[test]
    fn complete_body_escapes_etag() {
        assert_eq!(
            complete_body(&[(1, "\"a&b\"".into())]),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;a&amp;b&quot;</ETag></Part></CompleteMultipartUpload>"
        );
    }
}
