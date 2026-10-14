use super::{
    MediaInput, MultipartPart, MultipartSession, MultipartSignedPart, PreparedMedia,
    MAX_MEDIA_SIZE_BYTES, MULTIPART_PART_SIZE_BYTES,
};
use crate::error::ImError;
use helix_core::effect::{
    FileUploadProgressPolicy, FileUploadRange, FileUploadRequest, FileUploadUrls, HttpRequest,
};
use serde_json::{json, Value};

/// Build the Java multipart init request. Signed object-store credentials never
/// enter this request's durable representation; they exist only in the reply
/// route until the corresponding UploadFile effect is consumed.
pub fn multipart_init_request(
    java_api_base_url: &str,
    client_upload_id: &str,
    input: &MediaInput,
    target_bucket: &str,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    let base = checked_base(java_api_base_url)?;
    validate_size(input)?;
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    body_request(
        format!("{base}/oss/multipart/init"),
        headers,
        json!({
            "clientUploadId": client_upload_id,
            "bucket": target_bucket,
            "fileName": input.file_name,
            "contentType": input.content_type,
            "size": input.size,
            "sha256": input.sha256,
        }),
    )
}

pub fn multipart_parts_request(
    java_api_base_url: &str,
    session: &MultipartSession,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    let base = checked_base(java_api_base_url)?;
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    body_request(
        format!("{base}/oss/multipart/parts"),
        headers,
        common_body(session),
    )
}

pub fn multipart_sign_request(
    java_api_base_url: &str,
    session: &MultipartSession,
    part_numbers: &[u32],
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    let base = checked_base(java_api_base_url)?;
    validate_part_numbers(session, part_numbers).map_err(ImError::Parse)?;
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    let mut body = common_body(session);
    body["partNumbers"] = json!(part_numbers);
    body_request(format!("{base}/oss/multipart/sign"), headers, body)
}

pub fn multipart_complete_request(
    java_api_base_url: &str,
    session: &MultipartSession,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    let base = checked_base(java_api_base_url)?;
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    body_request(
        format!("{base}/oss/multipart/complete"),
        headers,
        common_body(session),
    )
}

/// Build one range upload. `size` remains the complete local file size while
/// the range controls the bytes read and therefore the host Content-Length.
pub fn multipart_upload_request(
    input: &MediaInput,
    session: &MultipartSession,
    signed: &MultipartSignedPart,
    target: &super::UploadTarget,
) -> Result<FileUploadRequest, ImError> {
    validate_size(input)?;
    let length = input
        .part_size(signed.part_number)
        .ok_or_else(|| ImError::Parse("multipart part number out of range".to_string()))?;
    if session.part_size != MULTIPART_PART_SIZE_BYTES {
        return Err(ImError::Parse(
            "multipart partSize is not 8 MiB".to_string(),
        ));
    }
    let offset = (u64::from(signed.part_number) - 1) * session.part_size;
    let urls = FileUploadUrls::new(signed.url.clone(), session.uri.clone())
        .map_err(|reason| ImError::Parse(format!("multipart signed URL invalid: {reason}")))?
        .with_progress_policy(progress_policy(target))
        .with_range(FileUploadRange { offset, length });
    Ok(FileUploadRequest {
        local_path: input.local_path.clone(),
        object_key: session.object_key.clone(),
        method: "PUT".to_string(),
        urls,
        headers: signed.headers.clone(),
        content_type: Some(input.content_type.clone()),
        size: Some(input.size),
    })
}

pub fn parse_multipart_init_reply(
    bytes: &[u8],
    pending: &super::PendingMediaPrepare,
) -> Result<MultipartSession, String> {
    let root = response_object(bytes, "Java multipart init")?;
    require_success(&root, "Java multipart init")?;
    if root.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Java multipart init protocolVersion must be 1".to_string());
    }
    let status = required_string(&root, "status")?;
    if !matches!(status.as_str(), "uploading" | "completed") {
        return Err("Java multipart init status must be uploading or completed".to_string());
    }
    let upload_id = required_string(&root, "uploadId")?;
    let file_id = required_string(&root, "fileId")?;
    let bucket = required_string(&root, "bucket")?;
    if bucket != pending.target.bucket() {
        return Err("Java multipart init bucket conflicts with upload target".to_string());
    }
    let object_key = required_string(&root, "objectKey")?;
    let uri = required_string(&root, "uri")?;
    if object_key != format!("media/{file_id}") || uri != object_key {
        return Err("Java multipart init objectKey/uri conflicts with fileId".to_string());
    }
    let part_size = root
        .get("partSize")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Java multipart init partSize must be integer".to_string())?;
    if part_size != MULTIPART_PART_SIZE_BYTES {
        return Err("Java multipart init partSize must be 8388608".to_string());
    }
    if root.get("size").and_then(Value::as_u64) != Some(pending.input.size)
        || root.get("sha256").and_then(Value::as_str) != Some(pending.input.sha256.as_str())
    {
        return Err("Java multipart init size/sha256 conflicts with mediaInput".to_string());
    }
    Ok(MultipartSession {
        temporary_id: pending.temporary_id.clone(),
        channel_id: pending.channel_id,
        target: pending.target.clone(),
        input: pending.input.clone(),
        client_upload_id: super::client_upload_id(&pending.temporary_id.0, &pending.target),
        upload_id,
        file_id,
        bucket,
        object_key,
        uri,
        part_size,
        status,
        parts: Vec::new(),
    })
}

pub fn parse_multipart_parts_reply(
    bytes: &[u8],
    session: &MultipartSession,
) -> Result<(String, Vec<MultipartPart>), String> {
    let root = response_object(bytes, "Java multipart parts")?;
    require_success(&root, "Java multipart parts")?;
    if required_string(&root, "uploadId")? != session.upload_id {
        return Err("Java multipart parts uploadId mismatch".to_string());
    }
    let status = required_string(&root, "status")?;
    if !matches!(status.as_str(), "uploading" | "completed") {
        return Err("Java multipart parts status must be uploading or completed".to_string());
    }
    let raw_parts = root
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| "Java multipart parts must be array".to_string())?;
    let part_count = session.input.part_count();
    if !(1..=128).contains(&part_count) {
        return Err("multipart part count must be in 1..=128".to_string());
    }
    if raw_parts.len() > part_count as usize {
        return Err("Java multipart parts exceeds the bounded part count".to_string());
    }
    // The protocol bounds a file at 1 GiB / 8 MiB = 128 parts. A fixed
    // presence table keeps duplicate detection and validation linear in the
    // remote list size during every recovery reconciliation.
    let mut seen = [false; 128];
    let mut parts = Vec::with_capacity(raw_parts.len());
    for raw in raw_parts {
        let part_number = raw
            .get("partNumber")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Java multipart partNumber must be u32".to_string())?;
        let size = raw
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Java multipart part size must be integer".to_string())?;
        let etag = raw
            .get("etag")
            .and_then(Value::as_str)
            .filter(|etag| !etag.trim().is_empty())
            .ok_or_else(|| "Java multipart part etag must be non-empty".to_string())?;
        let expected = session
            .input
            .part_size(part_number)
            .ok_or_else(|| "Java multipart partNumber out of range".to_string())?;
        let seen_index = usize::try_from(part_number - 1)
            .map_err(|_| "Java multipart partNumber out of range".to_string())?;
        if seen[seen_index] {
            return Err("Java multipart parts contains duplicate partNumber".to_string());
        }
        if size != expected {
            return Err("Java multipart part size conflicts with mediaInput".to_string());
        }
        seen[seen_index] = true;
        parts.push(MultipartPart {
            part_number,
            size,
            etag: etag.to_string(),
        });
    }
    parts.sort_by_key(|part| part.part_number);
    Ok((status, parts))
}

pub fn parse_multipart_sign_reply(
    bytes: &[u8],
    session: &MultipartSession,
    expected: &[u32],
) -> Result<Vec<MultipartSignedPart>, String> {
    validate_part_numbers(session, expected)?;
    let root = response_object(bytes, "Java multipart sign")?;
    require_success(&root, "Java multipart sign")?;
    if required_string(&root, "uploadId")? != session.upload_id {
        return Err("Java multipart sign uploadId mismatch".to_string());
    }
    if root
        .get("expiresAt")
        .and_then(Value::as_u64)
        .is_none_or(|expires_at| expires_at == 0)
    {
        return Err("Java multipart sign expiresAt must be positive integer".to_string());
    }
    let raw_parts = root
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| "Java multipart sign parts must be array".to_string())?;
    if raw_parts.len() > expected.len() {
        return Err("Java multipart sign returned more parts than requested".to_string());
    }
    let mut signed = Vec::with_capacity(raw_parts.len());
    for raw in raw_parts {
        let part_number = raw
            .get("partNumber")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Java multipart sign partNumber must be u32".to_string())?;
        if !expected.contains(&part_number)
            || signed
                .iter()
                .any(|p: &MultipartSignedPart| p.part_number == part_number)
        {
            return Err("Java multipart sign contains duplicate or unexpected part".to_string());
        }
        let url = raw
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| is_http_url(url))
            .ok_or_else(|| "Java multipart sign url must use http or https".to_string())?;
        let headers = raw
            .get("headers")
            .and_then(Value::as_object)
            .ok_or_else(|| "Java multipart sign headers must be object".to_string())?
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|value| (name.clone(), value.to_string()))
                    .ok_or_else(|| format!("Java multipart sign header {name} must be string"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        signed.push(MultipartSignedPart {
            part_number,
            url: url.to_string(),
            headers,
        });
    }
    if signed.len() != expected.len()
        || expected
            .iter()
            .any(|number| !signed.iter().any(|part| part.part_number == *number))
    {
        return Err("Java multipart sign did not return every requested part".to_string());
    }
    signed.sort_by_key(|part| part.part_number);
    Ok(signed)
}

pub fn parse_multipart_complete_reply(
    bytes: &[u8],
    session: &MultipartSession,
) -> Result<PreparedMedia, String> {
    let root = response_object(bytes, "Java multipart complete")?;
    require_success(&root, "Java multipart complete")?;
    if required_string(&root, "status")? != "completed"
        || required_string(&root, "uploadId")? != session.upload_id
    {
        return Err("Java multipart complete must return completed matching uploadId".to_string());
    }
    let file_id = required_string(&root, "fileId")?;
    let bucket = required_string(&root, "bucket")?;
    let object_key = required_string(&root, "objectKey")?;
    let uri = required_string(&root, "uri")?;
    if file_id != session.file_id
        || bucket != session.bucket
        || object_key != session.object_key
        || uri != session.uri
        || root.get("size").and_then(Value::as_u64) != Some(session.input.size)
        || root.get("sha256").and_then(Value::as_str) != Some(session.input.sha256.as_str())
    {
        return Err("Java multipart complete stable reference conflicts with init".to_string());
    }
    Ok(PreparedMedia {
        file_id,
        bucket,
        method: "MULTIPART".to_string(),
        // The complete response is a stable reference, never a signed URL.
        url: uri.clone(),
        uri,
        object_key,
        headers: Vec::new(),
    })
}

fn response_object(bytes: &[u8], operation: &str) -> Result<Value, String> {
    let raw = crate::http_envelope::unwrap_success_envelope(bytes, operation)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&raw).map_err(|error| format!("{operation} response: {error}"))
}

fn require_success(root: &Value, operation: &str) -> Result<(), String> {
    if root.get("isSuccess").and_then(Value::as_bool) != Some(true) {
        return Err(format!("{operation} isSuccess must be true"));
    }
    Ok(())
}

fn required_string(root: &Value, key: &str) -> Result<String, String> {
    root.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("multipart response {key} must be non-empty string"))
}

fn common_body(session: &MultipartSession) -> Value {
    json!({
        "clientUploadId": session.client_upload_id,
        "uploadId": session.upload_id,
        "bucket": session.bucket,
        "contentType": session.input.content_type,
        "size": session.input.size,
        "sha256": session.input.sha256,
    })
}

fn body_request(
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
) -> Result<HttpRequest, ImError> {
    let body = serde_json::to_vec(&body).map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(HttpRequest {
        method: "POST".to_string(),
        url,
        headers,
        body: Some(bytes::Bytes::from(body)),
    })
}

fn validate_size(input: &MediaInput) -> Result<(), ImError> {
    if input.size <= super::MULTIPART_THRESHOLD_BYTES || input.size > MAX_MEDIA_SIZE_BYTES {
        return Err(ImError::Parse(
            "multipart size must be >100 MiB and <=1 GiB".to_string(),
        ));
    }
    Ok(())
}

fn validate_part_numbers(session: &MultipartSession, numbers: &[u32]) -> Result<(), String> {
    if numbers.is_empty() || numbers.len() > 8 {
        return Err("multipart sign partNumbers must contain 1..=8 parts".to_string());
    }
    let mut seen = Vec::with_capacity(numbers.len());
    for number in numbers {
        if session.input.part_size(*number).is_none() || seen.contains(number) {
            return Err("multipart sign partNumbers must be unique and in range".to_string());
        }
        seen.push(*number);
    }
    Ok(())
}

fn checked_base(value: &str) -> Result<&str, ImError> {
    let value = value.trim_end_matches('/');
    if value.is_empty() {
        return Err(ImError::Parse(
            "Java API base is required for media upload".to_string(),
        ));
    }
    if !is_http_url(value) {
        return Err(ImError::Parse(
            "Java API base must use http or https".to_string(),
        ));
    }
    Ok(value)
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn progress_policy(target: &super::UploadTarget) -> FileUploadProgressPolicy {
    match target {
        super::UploadTarget::TemplateImage => FileUploadProgressPolicy::Disabled,
        _ => FileUploadProgressPolicy::PercentStep(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::upload_props::{PendingMediaPrepare, UploadTarget, MULTIPART_THRESHOLD_BYTES};
    use crate::state::{ChannelId, TemporaryId};

    fn pending(size: u64) -> PendingMediaPrepare {
        PendingMediaPrepare {
            temporary_id: TemporaryId("tmp".to_string()),
            channel_id: ChannelId::from_str("ch00000000000000000000000a").unwrap(),
            target: UploadTarget::File,
            input: MediaInput {
                local_path: "/tmp/a".to_string(),
                file_name: "a".to_string(),
                content_type: "application/octet-stream".to_string(),
                size,
                sha256: "a".repeat(64),
            },
        }
    }

    #[test]
    fn range_keeps_full_size_and_uses_part_length() {
        let p = pending(MULTIPART_THRESHOLD_BYTES + 1);
        let session = MultipartSession {
            temporary_id: p.temporary_id.clone(),
            channel_id: p.channel_id,
            target: p.target.clone(),
            input: p.input.clone(),
            client_upload_id: "tmp:file".to_string(),
            upload_id: "up".to_string(),
            file_id: "f".to_string(),
            bucket: "attachment".to_string(),
            object_key: "media/f".to_string(),
            uri: "media/f".to_string(),
            part_size: MULTIPART_PART_SIZE_BYTES,
            status: "uploading".to_string(),
            parts: Vec::new(),
        };
        let req = multipart_upload_request(
            &p.input,
            &session,
            &MultipartSignedPart {
                part_number: 13,
                url: "https://oss/part".to_string(),
                headers: Vec::new(),
            },
            &p.target,
        )
        .unwrap();
        assert_eq!(req.size, Some(p.input.size));
        assert_eq!(
            req.range(),
            Some(FileUploadRange {
                offset: MULTIPART_PART_SIZE_BYTES * 12,
                length: MULTIPART_THRESHOLD_BYTES - MULTIPART_PART_SIZE_BYTES * 12 + 1,
            })
        );
    }
}
