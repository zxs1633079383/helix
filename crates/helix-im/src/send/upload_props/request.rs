use super::UploadTarget;
use crate::ImError;
use helix_core::effect::{FileUploadRequest, FileUploadUrls};
use serde_json::Value;

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
            .map_err(|reason| ImError::Parse(format!("upload target invalid urls: {reason}")))?,
        headers,
        content_type: optional_string(node, upload, "contentType")?,
        size: optional_u64(node, upload, "size")?,
    })
}

fn target_node<'a>(props: &'a Value, target: &UploadTarget) -> Result<&'a Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get("images")
            .and_then(Value::as_array)
            .and_then(|images| images.get(*index))
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
