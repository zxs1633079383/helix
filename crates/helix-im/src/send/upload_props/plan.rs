use super::{MediaInput, PlannedMedia, UploadPlan, UploadTarget};
use crate::ImError;
use serde_json::{json, Map, Value};

const MEDIA_INPUT_FIELDS: [&str; 5] = ["localPath", "fileName", "contentType", "size", "sha256"];
const GO_OWNED_FIELDS: [&str; 12] = [
    "uploadId",
    "fileId",
    "method",
    "url",
    "publicUrl",
    "objectKey",
    "headers",
    "status",
    "state",
    "userId",
    "teamId",
    "etag",
];

pub fn build_upload_plan(
    msg_type: &str,
    message: &str,
    mut props: Value,
) -> Result<UploadPlan, ImError> {
    let media = match msg_type {
        "rich" => build_rich_plan(&mut props)?,
        "file" | "FILE" => build_file_plan(message, &mut props)?,
        _ => Vec::new(),
    };
    let targets = media.iter().map(|item| item.target.clone()).collect();
    Ok(UploadPlan {
        props,
        targets,
        media,
    })
}

fn build_rich_plan(props: &mut Value) -> Result<Vec<PlannedMedia>, ImError> {
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

    images
        .iter_mut()
        .enumerate()
        .map(|(index, image)| {
            extract_media(image, "rich image").map(|input| PlannedMedia {
                target: UploadTarget::RichImage { index },
                input,
            })
        })
        .collect()
}

fn build_file_plan(message: &str, props: &mut Value) -> Result<Vec<PlannedMedia>, ImError> {
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
    let input = extract_media(file, "file")?;
    Ok(vec![PlannedMedia {
        target: UploadTarget::File,
        input,
    }])
}

fn extract_media(node: &mut Value, label: &str) -> Result<MediaInput, ImError> {
    reject_go_owned_fields(node, label)?;
    let object = node
        .as_object_mut()
        .ok_or_else(|| ImError::Parse(format!("{label} must be object")))?;
    let media_input = object
        .remove("mediaInput")
        .ok_or_else(|| ImError::Parse(format!("{label} requires mediaInput")))?;
    let input = media_input
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput must be object")))?;
    validate_media_input_keys(input, label)?;

    let local_path = required_string(input, "localPath", label)?;
    let file_name = required_string(input, "fileName", label)?;
    let content_type = required_string(input, "contentType", label)?;
    let size = input
        .get("size")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.size must be positive u64")))?;
    let sha256 = input
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.sha256 must be string")))?
        .to_string();

    object.insert("upload".to_string(), json!({"status": "preparing"}));
    Ok(MediaInput {
        local_path,
        file_name,
        content_type,
        size,
        sha256,
    })
}

fn reject_go_owned_fields(value: &Value, label: &str) -> Result<(), ImError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if GO_OWNED_FIELDS.contains(&key.as_str()) {
                    return Err(ImError::Parse(format!(
                        "{label} must not supply Go-owned field {key}"
                    )));
                }
                reject_go_owned_fields(child, label)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_go_owned_fields(item, label)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_media_input_keys(input: &Map<String, Value>, label: &str) -> Result<(), ImError> {
    if let Some(key) = input
        .keys()
        .find(|key| !MEDIA_INPUT_FIELDS.contains(&key.as_str()))
    {
        return Err(ImError::Parse(format!(
            "{label} mediaInput contains unsupported field {key}"
        )));
    }
    Ok(())
}

fn required_string(input: &Map<String, Value>, key: &str, label: &str) -> Result<String, ImError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.{key} must be non-empty string")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uppercase_file_extracts_opaque_media_input() {
        let plan = build_upload_plan(
            "FILE",
            "",
            json!({
                "file": {
                    "mediaInput": {
                        "localPath": "/tmp/report.pdf",
                        "fileName": "report.pdf",
                        "contentType": "application/pdf",
                        "size": 9,
                        "sha256": "abc"
                    }
                }
            }),
        )
        .expect("uppercase FILE should produce a prepare plan");

        assert_eq!(plan.targets, vec![UploadTarget::File]);
        assert_eq!(plan.media[0].input.local_path, "/tmp/report.pdf");
        assert!(plan.props["file"].get("mediaInput").is_none());
        assert_eq!(plan.props["file"]["upload"]["status"], "preparing");
    }
}
