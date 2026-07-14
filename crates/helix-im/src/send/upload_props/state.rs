use super::{UploadSuccess, UploadTarget};
use crate::ImError;
use serde_json::{json, Value};

pub fn parse_upload_success_reply(bytes: &[u8]) -> Result<UploadSuccess, String> {
    let parsed: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("upload reply: {error}"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "upload reply must be object".to_string())?;
    let required = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("upload reply field {key} must be non-empty string"))
    };
    let etag = object
        .get("etag")
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "upload reply field etag must be string".to_string())
        })
        .transpose()?;
    Ok(UploadSuccess {
        object_key: required("objectKey")?,
        // 兼容 wire 的 `url` 已收紧为长期可读地址，不得传入 PUT URL。
        public_url: required("url")?,
        etag,
    })
}

pub fn mark_upload_success(
    props: &mut Value,
    target: &UploadTarget,
    result: &UploadSuccess,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    let upload = node
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    upload.insert("status".to_string(), json!("success"));
    upload.remove("url");
    upload.remove("headers");
    upload.insert("objectKey".to_string(), json!(&result.object_key));
    upload.insert("publicUrl".to_string(), json!(&result.public_url));
    if let Some(etag) = &result.etag {
        upload.insert("etag".to_string(), json!(etag));
    }
    Ok(())
}

pub fn mark_upload_failed(
    props: &mut Value,
    target: &UploadTarget,
    _error: &str,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    let upload = node
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    upload.insert("status".to_string(), json!("failed"));
    Ok(())
}

fn target_node_mut<'a>(
    props: &'a mut Value,
    target: &UploadTarget,
) -> Result<&'a mut Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get_mut("images")
            .and_then(Value::as_array_mut)
            .and_then(|images| images.get_mut(*index))
            .ok_or_else(|| ImError::Parse("missing rich image target".to_string())),
        UploadTarget::File => props
            .get_mut("file")
            .ok_or_else(|| ImError::Parse("missing file target".to_string())),
    }
}
