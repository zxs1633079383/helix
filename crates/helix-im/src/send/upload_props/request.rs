use super::{MediaInput, PreparedMedia, UploadTarget};
use crate::ImError;
use helix_core::effect::{
    FileUploadProgressPolicy, FileUploadRequest, FileUploadUrls, HttpRequest,
};
use serde_json::{json, Value};

pub fn prepare_upload_request(
    java_api_base_url: &str,
    client_upload_id: &str,
    input: &MediaInput,
    target: &UploadTarget,
    mut headers: Vec<(String, String)>,
) -> Result<HttpRequest, ImError> {
    let java_api_base_url = require_java_api_base_url(java_api_base_url)?;
    headers.insert(
        0,
        ("Content-Type".to_string(), "application/json".to_string()),
    );
    let body = serde_json::to_vec(&json!({
        "mediaUpload": true,
        "clientUploadId": client_upload_id,
        "bucket": target.bucket(),
        "fileName": input.file_name,
        "contentType": input.content_type,
        "size": input.size,
        "sha256": input.sha256,
    }))
    .map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(HttpRequest {
        method: "POST".to_string(),
        url: format!("{java_api_base_url}/oss/getPresignedUrl"),
        headers,
        body: Some(bytes::Bytes::from(body)),
    })
}

pub fn parse_prepare_reply(
    bytes: &[u8],
    input: &MediaInput,
    target: &UploadTarget,
) -> Result<PreparedMedia, String> {
    let raw = crate::http_envelope::unwrap_success_envelope(bytes, "Java media prepare")
        .map_err(|error| error.to_string())?;
    let root: Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("Java media prepare response: {error}"))?;
    if root.get("isSuccess").and_then(Value::as_bool) != Some(true) {
        return Err("Java media prepare isSuccess must be true".to_string());
    }
    let data = root
        .as_object()
        .ok_or_else(|| "Java media prepare response must be object".to_string())?;
    let required = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("Java media prepare {key} must be non-empty string"))
    };
    let method = required("method")?;
    if method != "PUT" {
        return Err("Java media prepare method must be PUT".to_string());
    }
    let url = required("url")?;
    if !is_http_url(&url) {
        return Err("Java media prepare url must use http or https".to_string());
    }
    let file_id = required("fileId")?;
    let bucket = required("bucket")?;
    if bucket != target.bucket() {
        return Err("Java media prepare bucket conflicts with upload target".to_string());
    }
    let object_key = required("objectKey")?;
    let expected_object_key = format!("media/{file_id}");
    if object_key != expected_object_key {
        return Err("Java media prepare objectKey conflicts with fileId".to_string());
    }
    let uri = required("uri")?;
    if uri != object_key {
        return Err("Java media prepare uri conflicts with objectKey".to_string());
    }
    let headers = data
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| "Java media prepare headers must be object".to_string())?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|header| (name.clone(), header.to_string()))
                .ok_or_else(|| format!("Java media prepare header {name} must be string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let content_type = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value)
        .ok_or_else(|| "Java media prepare headers missing Content-Type".to_string())?;
    if content_type != &input.content_type {
        return Err("Java media prepare Content-Type conflicts with mediaInput".to_string());
    }
    let forbid_overwrite = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-oss-forbid-overwrite"))
        .map(|(_, value)| value)
        .ok_or_else(|| "Java media prepare headers missing x-oss-forbid-overwrite".to_string())?;
    if forbid_overwrite != "true" {
        return Err("Java media prepare x-oss-forbid-overwrite must be true".to_string());
    }
    Ok(PreparedMedia {
        // 复用既有 durable `upload_id` 槽保存幂等文件标识，不再等待 Java complete 票据。
        upload_id: file_id.clone(),
        file_id,
        bucket,
        method,
        url,
        uri,
        object_key,
        headers,
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
        // helix-core 的第二个只读引用槽保持 ABI 不变；媒体链路在这里承载 bucket 内 uri。
        urls: FileUploadUrls::new(prepared.url.clone(), prepared.uri.clone())
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

pub fn client_upload_id(temporary_id: &str, target: &UploadTarget) -> String {
    match target {
        UploadTarget::RichImage { index } => format!("{temporary_id}:rich:{index}"),
        UploadTarget::File => format!("{temporary_id}:file"),
    }
}

fn require_java_api_base_url(value: &str) -> Result<&str, ImError> {
    checked_java_api_base_url(value).map_err(ImError::Parse)
}

pub fn validate_java_api_base_url(value: &str) -> Result<(), ImError> {
    require_java_api_base_url(value).map(|_| ())
}

fn checked_java_api_base_url(value: &str) -> Result<&str, String> {
    let value = value.trim_end_matches('/');
    if value.is_empty() {
        return Err(
            "Java API base is required for media upload; frontend must inject restHost".to_string(),
        );
    }
    if !is_http_url(value) {
        return Err("Java API base must use http or https".to_string());
    }
    Ok(value)
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
