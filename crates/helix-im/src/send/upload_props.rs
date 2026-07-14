use crate::state::TemporaryId;
use crate::ImError;
use helix_core::effect::{FileUploadRequest, FileUploadUrls, SqlValue, StorageOp, UpsertSpec};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UploadTarget {
    RichImage { index: usize },
    File,
}

#[derive(Debug, Clone)]
pub struct UploadPlan {
    pub props: Value,
    pub targets: Vec<UploadTarget>,
}

#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub temporary_id: TemporaryId,
    pub target: UploadTarget,
}

#[derive(Debug, Clone)]
pub struct UploadSuccess {
    pub object_key: String,
    pub public_url: String,
    pub etag: Option<String>,
}

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

pub fn build_upload_plan(
    msg_type: &str,
    message: &str,
    mut props: Value,
) -> Result<UploadPlan, ImError> {
    match msg_type {
        "rich" => build_rich_plan(&mut props).map(|targets| UploadPlan { props, targets }),
        "file" => build_file_plan(message, &mut props).map(|targets| UploadPlan { props, targets }),
        _ => Ok(UploadPlan {
            props,
            targets: Vec::new(),
        }),
    }
}

fn build_rich_plan(props: &mut Value) -> Result<Vec<UploadTarget>, ImError> {
    if props.get("file").is_some() {
        return Err(ImError::Parse(
            "rich message must not contain props.file".to_string(),
        ));
    }
    match props.get("textPosition").and_then(Value::as_str) {
        Some("top" | "bottom") => {}
        _ => {
            return Err(ImError::Parse(
                "rich message requires textPosition top/bottom".to_string(),
            ))
        }
    }
    let images = props
        .get_mut("images")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ImError::Parse("rich message requires props.images".to_string()))?;
    if images.is_empty() {
        return Err(ImError::Parse(
            "rich message requires at least one image".to_string(),
        ));
    }

    let mut targets = Vec::with_capacity(images.len());
    for (index, image) in images.iter_mut().enumerate() {
        validate_upload_node(image, "rich image")?;
        set_status(image, "uploading")?;
        targets.push(UploadTarget::RichImage { index });
    }
    Ok(targets)
}

fn build_file_plan(message: &str, props: &mut Value) -> Result<Vec<UploadTarget>, ImError> {
    if !message.trim().is_empty() {
        return Err(ImError::Parse(
            "file message must not contain message text".to_string(),
        ));
    }
    if props.get("images").is_some() {
        return Err(ImError::Parse(
            "file message must not contain props.images".to_string(),
        ));
    }
    let file = props
        .get_mut("file")
        .ok_or_else(|| ImError::Parse("file message requires props.file".to_string()))?;
    validate_upload_node(file, "file")?;
    set_status(file, "uploading")?;
    Ok(vec![UploadTarget::File])
}

fn validate_upload_node(node: &Value, label: &str) -> Result<(), ImError> {
    if node
        .get("localPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        return Err(ImError::Parse(format!("{label} requires localPath")));
    }
    let upload = node
        .get("upload")
        .ok_or_else(|| ImError::Parse(format!("{label} requires upload")))?;
    for key in ["method", "url", "publicUrl", "objectKey"] {
        if upload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err(ImError::Parse(format!("{label} upload requires {key}")));
        }
    }
    if !upload.get("headers").map(Value::is_object).unwrap_or(false) {
        return Err(ImError::Parse(format!(
            "{label} upload.headers must be object"
        )));
    }
    Ok(())
}

fn set_status(node: &mut Value, status: &str) -> Result<(), ImError> {
    node.get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?
        .insert("status".to_string(), json!(status));
    Ok(())
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

/// 生成可落库/进入 posts.create 的 props 副本。
///
/// PUT URL 与签名 headers 只留在进程内的 PendingSend，任何持久化入口都必须使用本副本。
pub fn sanitized_for_persistence(props: &Value) -> Value {
    let mut sanitized = props.clone();
    if let Some(images) = sanitized.get_mut("images").and_then(Value::as_array_mut) {
        for image in images {
            strip_write_credentials(image);
        }
    }
    if let Some(file) = sanitized.get_mut("file") {
        strip_write_credentials(file);
    }
    sanitized
}

fn strip_write_credentials(node: &mut Value) {
    if let Some(upload) = node.get_mut("upload").and_then(Value::as_object_mut) {
        upload.remove("url");
        upload.remove("headers");
    }
}

/// 上传 props 的统一持久化闸门；解析异常同样 fail-closed 为无凭据空对象。
pub fn props_persist_op(temporary_id: &TemporaryId, props_json: String) -> StorageOp {
    let props_json = serde_json::from_str::<Value>(&props_json)
        .map(|props| sanitized_for_persistence(&props))
        .and_then(|props| serde_json::to_string(&props))
        .unwrap_or_else(|_| "{}".to_string());
    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text(temporary_id.0.clone()),
            ),
            ("props".to_string(), SqlValue::Text(props_json)),
        ]],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
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
