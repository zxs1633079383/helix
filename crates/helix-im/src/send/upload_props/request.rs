use super::{MediaInput, PendingMediaPut, PreparedMedia, TrustedMediaReference, UploadTarget};
use crate::ImError;
use helix_core::effect::{
    FileUploadProgressPolicy, FileUploadRequest, FileUploadUrls, HttpRequest,
};
use serde_json::{json, Value};

pub fn prepare_upload_request(
    api_base_url: &str,
    channel_id: &str,
    input: &MediaInput,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    let body = serde_json::to_vec(&json!({
        "channelId": channel_id,
        "fileName": input.file_name,
        "contentType": input.content_type,
        "size": input.size,
        "sha256": input.sha256,
    }))
    .map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(HttpRequest {
        method: "POST".to_string(),
        url: format!("{api_base_url}/media/upload/prepare"),
        headers,
        body: Some(bytes::Bytes::from(body)),
    })
}

pub fn complete_upload_request(
    api_base_url: &str,
    upload_id: &str,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    let body = serde_json::to_vec(&json!({"uploadId": upload_id}))
        .map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(HttpRequest {
        method: "POST".to_string(),
        url: format!("{api_base_url}/media/upload/complete"),
        headers,
        body: Some(bytes::Bytes::from(body)),
    })
}

pub fn parse_prepare_reply(
    bytes: &[u8],
    input: &MediaInput,
    channel_id: &str,
) -> Result<PreparedMedia, String> {
    let raw = crate::http_envelope::unwrap_success_envelope(bytes, "media prepare")
        .map_err(|error| error.to_string())?;
    let root: Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("media prepare CommonRes: {error}"))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "media prepare CommonRes.status must be string".to_string())?;
    if status != "SUCCESS" {
        return Err(format!("media prepare CommonRes status is {status}"));
    }
    let data = root
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "media prepare CommonRes.data must be object".to_string())?;
    let required = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("media prepare data.{key} must be non-empty string"))
    };
    let method = required("method")?;
    if method != "PUT" {
        return Err("media prepare data.method must be PUT".to_string());
    }
    let url = required("url")?;
    let public_url = required("publicUrl")?;
    if !is_http_url(&url) || !is_http_url(&public_url) {
        return Err("media prepare URLs must use http or https".to_string());
    }
    if let Some(size) = data.get("size") {
        if size.as_u64() != Some(input.size) {
            return Err("media prepare data.size conflicts with mediaInput".to_string());
        }
    }
    if let Some(content_type) = data.get("contentType") {
        if content_type.as_str() != Some(input.content_type.as_str()) {
            return Err("media prepare data.contentType conflicts with mediaInput".to_string());
        }
    }
    if let Some(response_channel) = data.get("channelId") {
        if response_channel.as_str() != Some(channel_id) {
            return Err("media prepare data.channelId conflicts with request".to_string());
        }
    }
    let headers = data
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| "media prepare data.headers must be object".to_string())?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|header| (name.clone(), header.to_string()))
                .ok_or_else(|| format!("media prepare header {name} must be string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some((_, content_type)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        if content_type != &input.content_type {
            return Err("media prepare Content-Type header conflicts with mediaInput".to_string());
        }
    }
    Ok(PreparedMedia {
        upload_id: required("uploadId")?,
        file_id: required("fileId")?,
        method,
        url,
        public_url,
        object_key: required("objectKey")?,
        headers,
    })
}

pub fn parse_complete_reply(
    bytes: &[u8],
    pending: &PendingMediaPut,
) -> Result<TrustedMediaReference, String> {
    let raw = crate::http_envelope::unwrap_success_envelope(bytes, "media complete")
        .map_err(|error| error.to_string())?;
    let root: Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("media complete CommonRes: {error}"))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "media complete CommonRes.status must be string".to_string())?;
    if status != "SUCCESS" {
        return Err(format!("media complete CommonRes status is {status}"));
    }
    let data = root
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "media complete CommonRes.data must be object".to_string())?;
    let required = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("media complete data.{key} must be non-empty string"))
    };
    let state = required("state")?;
    if state != "completed" {
        return Err("media complete data.state must be completed".to_string());
    }
    let upload_id = required("uploadId")?;
    if upload_id != pending.prepared.upload_id {
        return Err("media complete data.uploadId conflicts with prepare".to_string());
    }
    let file_id = required("fileId")?;
    if file_id != pending.prepared.file_id {
        return Err("media complete data.fileId conflicts with prepare".to_string());
    }
    if let Some(id) = data.get("id") {
        if id.as_str() != Some(file_id.as_str()) {
            return Err("media complete data.id conflicts with fileId".to_string());
        }
    }
    let url = required("url")?;
    if url != pending.prepared.public_url {
        return Err("media complete data.url conflicts with prepare publicUrl".to_string());
    }
    let size = data
        .get("size")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "media complete data.size must be positive u64".to_string())?;
    if size != pending.input.size {
        return Err("media complete data.size conflicts with mediaInput".to_string());
    }
    let sha256 = required("sha256")?;
    if sha256 != pending.input.sha256 {
        return Err("media complete data.sha256 conflicts with mediaInput".to_string());
    }
    let content_type = required("contentType")?;
    if content_type != pending.input.content_type {
        return Err("media complete data.contentType conflicts with mediaInput".to_string());
    }
    let channel_id = required("channelId")?;
    if channel_id != pending.channel_id.as_str() {
        return Err("media complete data.channelId conflicts with prepare".to_string());
    }
    let file_name = required("fileName")?;
    if file_name != pending.input.file_name {
        return Err("media complete data.fileName conflicts with mediaInput".to_string());
    }

    Ok(TrustedMediaReference {
        file_id,
        url,
        file_name,
        content_type,
        size,
        sha256,
    })
}

pub fn media_upload_request(
    input: &MediaInput,
    prepared: &PreparedMedia,
    target: &UploadTarget,
) -> Result<FileUploadRequest, ImError> {
    Ok(FileUploadRequest {
        local_path: input.local_path.clone(),
        object_key: prepared.object_key.clone(),
        method: prepared.method.clone(),
        urls: FileUploadUrls::new(prepared.url.clone(), prepared.public_url.clone())
            .map_err(|reason| ImError::Parse(format!("media prepare invalid urls: {reason}")))?
            .with_progress_policy(progress_policy(target)),
        headers: prepared.headers.clone(),
        content_type: Some(input.content_type.clone()),
        size: Some(input.size),
    })
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

pub fn file_upload_request(
    props: &Value,
    target: &UploadTarget,
) -> Result<FileUploadRequest, ImError> {
    let node = target_node(props, target)?;
    let upload = node
        .get("upload")
        .and_then(Value::as_object)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    let local_path = node
        .get("localPath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("upload target missing localPath".to_string()))?;
    let method = upload
        .get("method")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("upload target missing method".to_string()))?;
    let upload_url = upload
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("upload target missing url".to_string()))?;
    let public_url = upload
        .get("publicUrl")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("upload target missing publicUrl".to_string()))?;
    let object_key = upload
        .get("objectKey")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("upload target missing objectKey".to_string()))?;
    let headers = upload
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| ImError::Parse("upload target missing headers".to_string()))?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|header| (name.clone(), header.to_string()))
                .ok_or_else(|| ImError::Parse(format!("upload header {name} must be string")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(FileUploadRequest {
        local_path: local_path.to_string(),
        object_key: object_key.to_string(),
        method: method.to_string(),
        urls: FileUploadUrls::new(upload_url.to_string(), public_url.to_string())
            .map_err(|reason| ImError::Parse(format!("upload target invalid urls: {reason}")))?
            .with_progress_policy(progress_policy(target)),
        headers,
        content_type: optional_string(node, upload, "contentType")?,
        size: optional_u64(node, upload, "size")?,
    })
}

fn progress_policy(target: &UploadTarget) -> FileUploadProgressPolicy {
    match target {
        UploadTarget::RichImage { .. } => FileUploadProgressPolicy::Disabled,
        UploadTarget::File => FileUploadProgressPolicy::PercentStep(5),
    }
}

fn target_node<'a>(props: &'a Value, target: &UploadTarget) -> Result<&'a Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get("files")
            .and_then(Value::as_array)
            .and_then(|files| files.get(*index))
            .ok_or_else(|| ImError::Parse("missing rich image target".to_string())),
        UploadTarget::File => props
            .get("file")
            .ok_or_else(|| ImError::Parse("missing file target".to_string())),
    }
}

fn optional_string(
    node: &Value,
    upload: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, ImError> {
    let Some(value) = node.get(key).or_else(|| upload.get(key)) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|text| Some(text.to_string()))
        .ok_or_else(|| ImError::Parse(format!("upload field {key} must be string")))
}

fn optional_u64(
    node: &Value,
    upload: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<u64>, ImError> {
    let Some(value) = node.get(key).or_else(|| upload.get(key)) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| ImError::Parse(format!("upload field {key} must be u64")))
}
