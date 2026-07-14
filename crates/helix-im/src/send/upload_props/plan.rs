use super::{UploadPlan, UploadTarget};
use crate::ImError;
use serde_json::{json, Value};

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
