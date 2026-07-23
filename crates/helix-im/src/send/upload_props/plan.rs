use super::{MediaInput, PlannedMedia, UploadPlan, UploadTarget};
use crate::ImError;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const MEDIA_INPUT_FIELDS: [&str; 5] = ["localPath", "fileName", "contentType", "size", "sha256"];
const MAX_MEDIA_SIZE: u64 = 64 << 20;
const MAX_RICH_IMAGES: usize = 9;
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
        "rich" | "IMAGE" => build_rich_plan(&mut props)?,
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
    if props.get("images").is_some() || props.get("textPosition").is_some() {
        return Err(ImError::Parse(
            "rich message must use props.files and imagePosition".to_string(),
        ));
    }
    match props.get("imagePosition").and_then(Value::as_str) {
        Some("top" | "bottom") => {}
        _ => {
            return Err(ImError::Parse(
                "rich message requires imagePosition top/bottom".to_string(),
            ))
        }
    }
    let files = props
        .get_mut("files")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ImError::Parse("rich message requires props.files".to_string()))?;
    if files.is_empty() {
        return Err(ImError::Parse(
            "rich message requires at least one image".to_string(),
        ));
    }
    if files.len() > MAX_RICH_IMAGES {
        return Err(ImError::Parse(format!(
            "rich message supports at most {MAX_RICH_IMAGES} images"
        )));
    }

    files
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

fn build_file_plan(_message: &str, props: &mut Value) -> Result<Vec<PlannedMedia>, ImError> {
    if props.get("files").is_some() {
        return Err(ImError::Parse(
            "file message must not contain props.files".to_string(),
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
    let file_name = validate_file_name(input, label)?;
    let content_type = validate_content_type(input, label)?;
    let size = input
        .get("size")
        .and_then(Value::as_u64)
        .filter(|value| (1..=MAX_MEDIA_SIZE).contains(value))
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{label} mediaInput.size must be u64 in 1..={MAX_MEDIA_SIZE}"
            ))
        })?;
    let sha256 = validate_sha256(input, label)?;

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

fn validate_file_name(input: &Map<String, Value>, label: &str) -> Result<String, ImError> {
    let value = input
        .get("fileName")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.fileName must be string")))?
        .trim();
    if value.is_empty()
        || value.len() > 255
        || value
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'/' | b'\\' | 0))
    {
        return Err(ImError::Parse(format!(
            "{label} mediaInput.fileName is invalid"
        )));
    }
    Ok(value.to_string())
}

fn validate_content_type(input: &Map<String, Value>, label: &str) -> Result<String, ImError> {
    let value = input
        .get("contentType")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.contentType must be string")))?
        .trim();
    if value.len() > 127 || !is_valid_media_type(value) {
        return Err(ImError::Parse(format!(
            "{label} mediaInput.contentType is invalid"
        )));
    }
    Ok(value.to_string())
}

fn validate_sha256(input: &Map<String, Value>, label: &str) -> Result<String, ImError> {
    let value = input
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse(format!("{label} mediaInput.sha256 must be string")))?;
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ImError::Parse(format!(
            "{label} mediaInput.sha256 must be 64 lowercase hex characters"
        )));
    }
    Ok(value.to_string())
}

// Mirrors Go mime.ParseMediaType's token/parameter grammar without adding an I/O dependency.
fn is_valid_media_type(value: &str) -> bool {
    let separator = value.find(';').unwrap_or(value.len());
    let base = &value[..separator];
    let mut rest = &value[separator..];
    if !is_valid_media_base(base.trim()) {
        return false;
    }

    let mut params: HashMap<String, String> = HashMap::new();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some(after_semicolon) = rest.strip_prefix(';') else {
            return false;
        };
        rest = after_semicolon.trim_start();
        if rest.is_empty() {
            return true;
        }

        let key_len = token_len(rest.as_bytes());
        if key_len == 0 {
            return false;
        }
        let key = rest[..key_len].to_ascii_lowercase();
        rest = rest[key_len..].trim_start();
        let Some(after_equals) = rest.strip_prefix('=') else {
            return false;
        };
        rest = after_equals.trim_start();
        let Some((param_value, remaining)) = consume_media_value(rest) else {
            return false;
        };
        if params
            .insert(key, param_value.clone())
            .is_some_and(|previous| previous != param_value)
        {
            return false;
        }
        rest = remaining;
    }
    true
}

fn is_valid_media_base(value: &str) -> bool {
    let bytes = value.as_bytes();
    let type_len = token_len(bytes);
    if type_len == 0 {
        return false;
    }
    if type_len == bytes.len() {
        return true;
    }
    if bytes[type_len] != b'/' {
        return false;
    }
    let subtype = &bytes[type_len + 1..];
    !subtype.is_empty() && token_len(subtype) == subtype.len()
}

fn consume_media_value(value: &str) -> Option<(String, &str)> {
    if let Some(quoted) = value.strip_prefix('"') {
        let bytes = quoted.as_bytes();
        let mut output = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'"' => {
                    return String::from_utf8(output)
                        .ok()
                        .map(|parsed| (parsed, &quoted[index + 1..]));
                }
                b'\r' | b'\n' => return None,
                b'\\' if index + 1 < bytes.len() && is_tspecial(bytes[index + 1]) => {
                    output.push(bytes[index + 1]);
                    index += 2;
                }
                byte => {
                    output.push(byte);
                    index += 1;
                }
            }
        }
        None
    } else {
        let len = token_len(value.as_bytes());
        (len > 0).then(|| (value[..len].to_string(), &value[len..]))
    }
}

fn token_len(value: &[u8]) -> usize {
    value
        .iter()
        .position(|byte| !is_token_char(*byte))
        .unwrap_or(value.len())
}

fn is_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
        )
}

fn is_tspecial(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')'
            | b'<'
            | b'>'
            | b'@'
            | b','
            | b';'
            | b':'
            | b'\\'
            | b'"'
            | b'/'
            | b'['
            | b']'
            | b'?'
            | b'='
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn file_props() -> Value {
        json!({
            "file": {
                "mediaInput": {
                    "localPath": "/tmp/report.pdf",
                    "fileName": "report.pdf",
                    "contentType": "application/pdf",
                    "size": 9,
                    "sha256": VALID_SHA256
                }
            }
        })
    }

    #[test]
    fn file_plan_preserves_optional_text() {
        let plan = build_upload_plan("FILE", "quarterly report", file_props())
            .expect("FILE optional text should not block upload planning");

        assert_eq!(plan.targets, vec![UploadTarget::File]);
    }

    #[test]
    fn file_plan_rejects_image_files_collection() {
        let mut props = file_props();
        props["files"] = json!([]);

        let error = build_upload_plan("FILE", "", props)
            .expect_err("FILE must keep the single props.file contract");
        assert!(error.to_string().contains("must not contain props.files"));
    }

    #[test]
    fn invalid_go_metadata_boundaries_are_rejected_before_planning() {
        let invalid = [
            ("fileName", json!("../report.pdf")),
            ("fileName", json!("report\\2026.pdf")),
            ("contentType", json!("not a mime")),
            ("size", json!((64_u64 << 20) + 1)),
            (
                "sha256",
                json!("ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"),
            ),
            ("sha256", json!("short")),
        ];

        for (field, value) in invalid {
            let mut props = file_props();
            props["file"]["mediaInput"][field] = value;
            let error = build_upload_plan("FILE", "", props)
                .expect_err(&format!("invalid {field} must fail closed"));
            assert!(error.to_string().contains(field), "{field}: {error}");
        }
    }

    #[test]
    fn go_metadata_trimming_and_upper_size_boundary_are_preserved() {
        let mut props = file_props();
        props["file"]["mediaInput"]["fileName"] = json!(" report.pdf ");
        props["file"]["mediaInput"]["contentType"] = json!(" application/pdf; charset=utf-8 ");
        props["file"]["mediaInput"]["size"] = json!(64_u64 << 20);

        let plan = build_upload_plan("FILE", "caption", props).expect("valid upper boundary");
        assert_eq!(plan.media[0].input.file_name, "report.pdf");
        assert_eq!(
            plan.media[0].input.content_type,
            "application/pdf; charset=utf-8"
        );
        assert_eq!(plan.media[0].input.size, 64_u64 << 20);
    }

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
                        "sha256": VALID_SHA256
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

    #[test]
    fn rich_image_count_is_bounded_to_nine() {
        let image = || {
            json!({
                "mediaInput": {
                    "localPath": "/tmp/a.png",
                    "fileName": "a.png",
                    "contentType": "image/png",
                    "size": 3,
                    "sha256": VALID_SHA256
                }
            })
        };
        let nine = Value::Array((0..9).map(|_| image()).collect());
        let plan = build_upload_plan(
            "IMAGE",
            "caption",
            json!({"imagePosition":"bottom", "files":nine}),
        )
        .expect("nine images are accepted");
        assert_eq!(plan.media.len(), 9);
        assert_eq!(plan.props["imagePosition"], "bottom");
        assert!(plan.props.get("images").is_none());
        assert!(plan.props.get("textPosition").is_none());

        let legacy_error = build_upload_plan(
            "IMAGE",
            "caption",
            json!({
                "imagePosition": "bottom",
                "files": [image()],
                "textPosition": "bottom",
                "images": [image()]
            }),
        )
        .expect_err("legacy image fields must not be persisted beside canonical fields");
        assert!(legacy_error
            .to_string()
            .contains("must use props.files and imagePosition"));

        let ten = Value::Array((0..10).map(|_| image()).collect());
        let error = build_upload_plan(
            "IMAGE",
            "caption",
            json!({"imagePosition":"bottom", "files":ten}),
        )
        .expect_err("ten images must fail closed");
        assert!(error.to_string().contains("at most 9"));
    }
}
