use crate::ImError;
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
pub struct UploadSuccess {
    pub object_key: String,
    pub url: Option<String>,
    pub etag: Option<String>,
}

pub fn build_upload_plan(
    msg_type: &str,
    message: &str,
    mut props: Value,
) -> Result<UploadPlan, ImError> {
    match msg_type {
        "rich" => build_rich_plan(&mut props),
        "file" => build_file_plan(message, &mut props),
        _ => Ok(UploadPlan {
            props,
            targets: Vec::new(),
        }),
    }
}

fn build_rich_plan(props: &mut Value) -> Result<UploadPlan, ImError> {
    if props.get("file").is_some() {
        return Err(ImError::Parse("rich message must not contain props.file".to_string()));
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
    Ok(UploadPlan {
        props: props.clone(),
        targets,
    })
}

fn build_file_plan(message: &str, props: &mut Value) -> Result<UploadPlan, ImError> {
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
    Ok(UploadPlan {
        props: props.clone(),
        targets: vec![UploadTarget::File],
    })
}

fn validate_upload_node(node: &Value, label: &str) -> Result<(), ImError> {
    if node.get("localPath").and_then(Value::as_str).unwrap_or("").is_empty() {
        return Err(ImError::Parse(format!("{label} requires localPath")));
    }
    let upload = node
        .get("upload")
        .ok_or_else(|| ImError::Parse(format!("{label} requires upload")))?;
    for key in ["method", "url", "objectKey"] {
        if upload.get(key).and_then(Value::as_str).unwrap_or("").is_empty() {
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
    node
        .get_mut("upload")
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
    upload.insert("objectKey".to_string(), json!(result.object_key));
    if let Some(url) = &result.url {
        upload.insert("url".to_string(), json!(url));
    }
    if let Some(etag) = &result.etag {
        upload.insert("etag".to_string(), json!(etag));
    }
    Ok(())
}

pub fn mark_upload_failed(
    props: &mut Value,
    target: &UploadTarget,
    error: &str,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    let upload = node
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    upload.insert("status".to_string(), json!("failed"));
    upload.insert("error".to_string(), json!(error));
    Ok(())
}

fn target_node_mut<'a>(props: &'a mut Value, target: &UploadTarget) -> Result<&'a mut Value, ImError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rich_accepts_images_and_preserves_frontend_fields() {
        let props = json!({
            "textPosition": "bottom",
            "images": [{
                "localPath": "/tmp/a.png",
                "name": "a.png",
                "width": 800,
                "upload": {
                    "status": "pending",
                    "method": "PUT",
                    "url": "https://upload",
                    "headers": {"x-oss": "1"},
                    "objectKey": "im/a.png"
                }
            }],
            "frontendCustom": {"keep": true}
        });

        let plan = build_upload_plan("rich", "hello", props).expect("rich image plan");

        assert_eq!(plan.targets, vec![UploadTarget::RichImage { index: 0 }]);
        assert_eq!(plan.props["images"][0]["upload"]["status"], "uploading");
        assert_eq!(plan.props["images"][0]["width"], 800);
        assert_eq!(plan.props["frontendCustom"]["keep"], true);
    }

    #[test]
    fn rich_rejects_file_field() {
        let err = build_upload_plan(
            "rich",
            "hello",
            json!({"textPosition":"top","images":[],"file":{"localPath":"/tmp/a.pdf"}}),
        )
        .unwrap_err();

        assert!(err.to_string().contains("rich"));
    }

    #[test]
    fn file_rejects_message_text() {
        let props = json!({
            "file": {
                "localPath": "/tmp/a.pdf",
                "upload": {"method":"PUT","url":"https://upload","headers":{},"objectKey":"im/a.pdf"}
            }
        });

        let err = build_upload_plan("file", "not allowed", props).unwrap_err();

        assert!(err.to_string().contains("file"));
    }

    #[test]
    fn mark_success_updates_only_target_upload_node() {
        let mut props = json!({
            "textPosition":"bottom",
            "images":[
                {"localPath":"/tmp/a.png","upload":{"status":"uploading","objectKey":"im/a.png"}},
                {"localPath":"/tmp/b.png","upload":{"status":"uploading","objectKey":"im/b.png"}}
            ],
            "frontendCustom": 7
        });

        mark_upload_success(
            &mut props,
            &UploadTarget::RichImage { index: 1 },
            &UploadSuccess {
                object_key: "im/b.png".to_string(),
                url: Some("https://cdn/b.png".to_string()),
                etag: Some("etag-b".to_string()),
            },
        )
        .expect("success mutation");

        assert_eq!(props["images"][0]["upload"]["status"], "uploading");
        assert_eq!(props["images"][1]["upload"]["status"], "success");
        assert_eq!(props["images"][1]["upload"]["etag"], "etag-b");
        assert_eq!(props["frontendCustom"], 7);
    }
}
