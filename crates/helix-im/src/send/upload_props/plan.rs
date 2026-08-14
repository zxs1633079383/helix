use super::{MediaInput, PlannedMedia, UploadPlan, UploadTarget};
use crate::ImError;
use serde_json::{json, Map, Value};

const MEDIA_INPUT_FIELDS: [&str; 5] = ["localPath", "fileName", "contentType", "size", "sha256"];
const MAX_MEDIA_SIZE: u64 = 64 << 20;
const MAX_RICH_MEDIA: usize = 9;
const TRANSIENT_REFERENCE_FIELDS: [&str; 10] = [
    "uploadId",
    "uploadToken",
    "method",
    "url",
    "publicUrl",
    "headers",
    "expiresAt",
    "status",
    "state",
    "etag",
];
const SERVER_OWNED_FIELDS: [&str; 19] = [
    "bucket",
    "id",
    "uri",
    "uploadId",
    "uploadToken",
    "fileId",
    "method",
    "url",
    "publicUrl",
    "stablePath",
    "objectKey",
    "headers",
    "expiresAt",
    "contentLength",
    "status",
    "state",
    "userId",
    "teamId",
    "etag",
];

/// 根据消息类型构造上传计划；稳定对象引用保持零上传。
pub fn build_upload_plan(
    msg_type: &str,
    message: &str,
    mut props: Value,
) -> Result<UploadPlan, ImError> {
    let media = match msg_type {
        "rich" | "RICH" => build_rich_plan(&mut props, true)?,
        "IMAGE" => build_rich_plan(&mut props, false)?,
        "file" | "FILE" | "AUDIO" | "VIDEO" => build_file_plan(message, &mut props)?,
        "TEMPLATE" => build_template_plan(&mut props)?,
        _ => Vec::new(),
    };
    let targets = media.iter().map(|item| item.target.clone()).collect();
    // 消息级进度是**按总字节加权**的 O(n) 单趟投影，只在规划这一次边界算一次；
    // 后续单文件推进走 O(1) 的 `apply_upload_state`，不在此重算（T076）。
    if !media.is_empty() {
        let weighted = UploadTarget::message_upload_progress(&props)?;
        props
            .as_object_mut()
            .ok_or_else(|| ImError::Parse("props must be object".to_string()))?
            .insert("uploadProgress".to_string(), json!(weighted));
    }
    Ok(UploadPlan {
        props,
        targets,
        media,
    })
}

/// 线性扫描富媒体项；RICH 接受图片和视频，兼容 IMAGE 只接受图片。
fn build_rich_plan(props: &mut Value, allow_video: bool) -> Result<Vec<PlannedMedia>, ImError> {
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
            "rich message requires at least one media item".to_string(),
        ));
    }
    if files.len() > MAX_RICH_MEDIA {
        return Err(ImError::Parse(format!(
            "rich message supports at most {MAX_RICH_MEDIA} media items"
        )));
    }

    let mut media = Vec::with_capacity(files.len());
    for (index, item) in files.iter_mut().enumerate() {
        let target = rich_target(item, index, allow_video)?;
        if is_stable_object_reference(item, "rich media")? {
            if item.get("bucket").and_then(Value::as_str) != Some(target.bucket()) {
                return Err(ImError::Parse(
                    "rich media bucket conflicts with contentType".to_string(),
                ));
            }
            continue;
        }
        media.push(PlannedMedia {
            target,
            input: extract_media(item, "rich media")?,
        });
    }
    Ok(media)
}

/// 以受校验的 MIME 主类型选择 RICH 的稳定 bucket 与上传目标。
fn rich_target(node: &Value, index: usize, allow_video: bool) -> Result<UploadTarget, ImError> {
    let content_type = node
        .pointer("/mediaInput/contentType")
        .or_else(|| node.get("contentType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| ImError::Parse("rich media requires contentType".to_string()))?;
    if content_type.starts_with("image/") {
        return Ok(UploadTarget::RichImage { index });
    }
    if allow_video && content_type.starts_with("video/") {
        return Ok(UploadTarget::RichVideo { index });
    }
    let expected = if allow_video {
        "image/* or video/*"
    } else {
        "image/*"
    };
    Err(ImError::Parse(format!(
        "rich media contentType must be {expected}"
    )))
}

/// 构造单文件计划；已有稳定对象引用不进入上传链。
fn build_file_plan(_message: &str, props: &mut Value) -> Result<Vec<PlannedMedia>, ImError> {
    if props.get("files").is_some() {
        return Err(ImError::Parse(
            "file message must not contain props.files".to_string(),
        ));
    }
    let file = props
        .get_mut("file")
        .ok_or_else(|| ImError::Parse("file message requires props.file".to_string()))?;
    if is_stable_object_reference(file, "file")? {
        return Ok(Vec::new());
    }
    let input = extract_media(file, "file")?;
    Ok(vec![PlannedMedia {
        target: UploadTarget::File,
        input,
    }])
}

/// 构造 TEMPLATE/IMAGE 的嵌套上传计划；纯文本模板保持零媒体 Effect。
fn build_template_plan(props: &mut Value) -> Result<Vec<PlannedMedia>, ImError> {
    let template = props
        .get_mut("template")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("TEMPLATE requires props.template".to_string()))?;
    match template.get("type").and_then(Value::as_str) {
        Some("TEXT") => return Ok(Vec::new()),
        Some("IMAGE") => {}
        _ => {
            return Err(ImError::Parse(
                "TEMPLATE requires type TEXT or IMAGE".to_string(),
            ))
        }
    }
    let file = template
        .get_mut("file")
        .ok_or_else(|| ImError::Parse("TEMPLATE/IMAGE requires props.template.file".to_string()))?;
    if is_stable_object_reference(file, "template image")? {
        return Ok(Vec::new());
    }
    Ok(vec![PlannedMedia {
        target: UploadTarget::TemplateImage,
        input: extract_media(file, "template image")?,
    }])
}

/// 识别不可伪造临时凭据的稳定对象描述符，并拒绝半完整引用。
fn is_stable_object_reference(node: &Value, label: &str) -> Result<bool, ImError> {
    let object = node
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{label} must be object")))?;
    if object.contains_key("mediaInput") {
        return Ok(false);
    }
    let has_reference_field = ["bucket", "id", "uri"]
        .iter()
        .any(|field| object.contains_key(*field));
    if !has_reference_field {
        return Ok(false);
    }
    if let Some(field) = TRANSIENT_REFERENCE_FIELDS
        .iter()
        .find(|field| object.contains_key(**field))
    {
        return Err(ImError::Parse(format!(
            "{label} stable reference must not contain transient field {field}"
        )));
    }
    let bucket = stable_reference_string(object, "bucket", label)?;
    let id = stable_reference_string(object, "id", label)?;
    let uri = stable_reference_string(object, "uri", label)?;
    if bucket.len() > 64
        || id.len() > 255
        || !uri.starts_with('/')
        || uri.starts_with("//")
        || uri.contains("..")
        || uri.contains(['?', '#', '\\'])
    {
        return Err(ImError::Parse(format!(
            "{label} stable object reference is invalid"
        )));
    }
    Ok(true)
}

/// 读取稳定引用的必填非空字符串字段。
fn stable_reference_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, ImError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{label} stable reference requires non-empty {field}"
            ))
        })
}

/// 提取并清除 Host 暂存输入，只保留可持久化媒体元数据。
fn extract_media(node: &mut Value, label: &str) -> Result<MediaInput, ImError> {
    reject_server_owned_fields(node, label)?;
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

    // mediaInput 与本地路径别名不进入公开 props；保留前端渲染发送中/失败卡片所需的
    // 非敏感 metadata。bucket 是现网 getFilePath 的必填合同，固定为 Java file bucket。
    for field in ["localUrl", "local_url", "localPath", "local_path"] {
        object.remove(field);
    }
    object.insert("bucket".to_string(), json!("file"));
    object.insert("name".to_string(), json!(&file_name));
    object.insert("contentType".to_string(), json!(&content_type));
    object.insert("size".to_string(), json!(size));
    object.insert("upload".to_string(), json!({"status": "preparing"}));
    // 逐文件绝对进度必须在乐观落库/首帧 Sending 之前就位（INV-01 / T076）。
    super::state::init_upload_state(object);
    Ok(MediaInput {
        local_path,
        file_name,
        content_type,
        size,
        sha256,
    })
}

fn reject_server_owned_fields(value: &Value, label: &str) -> Result<(), ImError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if SERVER_OWNED_FIELDS.contains(&key.as_str()) {
                    return Err(ImError::Parse(format!(
                        "{label} must not supply server-owned field {key}"
                    )));
                }
                reject_server_owned_fields(child, label)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_server_owned_fields(item, label)?;
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

// Java mediaUpload 的 HMAC 与 OSS metadata 都绑定规范化前的精确 Content-Type。
// v1 只接受无参数的 type/subtype，避免 Rust 接受 `; charset=...` 后被 Java 拒绝。
fn is_valid_media_type(value: &str) -> bool {
    let Some((media_type, subtype)) = value.split_once('/') else {
        return false;
    };
    !media_type.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && media_type
            .as_bytes()
            .iter()
            .all(|byte| is_media_token(*byte))
        && subtype.as_bytes().iter().all(|byte| is_media_token(*byte))
}

fn is_media_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn file_props() -> Value {
        json!({
            "file": {
                "localUrl": "file:///tmp/report.pdf",
                "local_path": "/tmp/report.pdf",
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
    fn invalid_server_metadata_boundaries_are_rejected_before_planning() {
        let invalid = [
            ("fileName", json!("../report.pdf")),
            ("fileName", json!("report\\2026.pdf")),
            ("contentType", json!("not a mime")),
            ("contentType", json!("application/pdf; charset=utf-8")),
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
    fn server_metadata_trimming_and_upper_size_boundary_are_preserved() {
        let mut props = file_props();
        props["file"]["mediaInput"]["fileName"] = json!(" report.pdf ");
        props["file"]["mediaInput"]["contentType"] = json!(" application/pdf ");
        props["file"]["mediaInput"]["size"] = json!(64_u64 << 20);

        let plan = build_upload_plan("FILE", "caption", props).expect("valid upper boundary");
        assert_eq!(plan.media[0].input.file_name, "report.pdf");
        assert_eq!(plan.media[0].input.content_type, "application/pdf");
        assert_eq!(plan.media[0].input.size, 64_u64 << 20);
    }

    #[test]
    fn client_cannot_inject_trusted_media_reference_fields() {
        for (field, value) in [
            ("id", json!("attacker-id")),
            ("uri", json!("https://evil.example/file")),
            ("stablePath", json!("/oss/media/object/attacker")),
            ("expiresAt", json!(u64::MAX)),
            ("contentLength", json!(9)),
        ] {
            let mut props = file_props();
            props["file"][field] = value;
            let error = build_upload_plan("FILE", "", props)
                .expect_err(&format!("{field} must remain Java/Helix owned"));
            assert!(
                error.to_string().contains(field),
                "unexpected {field} error: {error}"
            );
        }
    }

    #[test]
    fn uppercase_file_extracts_opaque_media_input() {
        let plan = build_upload_plan("FILE", "", file_props())
            .expect("uppercase FILE should produce a prepare plan");

        assert_eq!(plan.targets, vec![UploadTarget::File]);
        assert_eq!(plan.media[0].input.local_path, "/tmp/report.pdf");
        assert!(plan.props["file"].get("mediaInput").is_none());
        assert!(plan.props["file"].get("localUrl").is_none());
        assert!(plan.props["file"].get("local_path").is_none());
        assert_eq!(plan.props["file"]["upload"]["status"], "preparing");
    }

    /// AUDIO 和 VIDEO 与单文件共用真实 prepare/PUT/complete 状态机。
    #[test]
    fn audio_and_video_build_single_file_upload_plans() {
        for message_type in ["AUDIO", "VIDEO"] {
            let plan = build_upload_plan(message_type, "", file_props())
                .expect("audio/video must produce a prepare plan");
            assert_eq!(plan.targets, vec![UploadTarget::File]);
            assert!(plan.props["file"].get("mediaInput").is_none());
        }
    }

    /// RICH 混合媒体按 MIME 分流到 picture/attachment，且仍属于同一上传计划。
    #[test]
    fn rich_image_and_video_use_distinct_stable_buckets() {
        let props = json!({
            "imagePosition": "bottom",
            "files": [
                {"mediaInput": {
                    "localPath": "/tmp/a.png", "fileName": "a.png",
                    "contentType": "image/png", "size": 3, "sha256": VALID_SHA256
                }},
                {"mediaInput": {
                    "localPath": "/tmp/b.mp4", "fileName": "b.mp4",
                    "contentType": "video/mp4", "size": 9, "sha256": VALID_SHA256
                }}
            ]
        });

        let plan = build_upload_plan("RICH", "caption", props).expect("mixed rich plan");

        assert_eq!(
            plan.targets,
            vec![
                UploadTarget::RichImage { index: 0 },
                UploadTarget::RichVideo { index: 1 }
            ]
        );
        assert_eq!(plan.targets[0].bucket(), "picture");
        assert_eq!(plan.targets[1].bucket(), "attachment");
    }

    /// 兼容 IMAGE 不接受视频，避免旧图片消息悄然改变 wire 语义。
    #[test]
    fn legacy_image_rejects_video_items() {
        let props = json!({
            "imagePosition": "bottom",
            "files": [{"mediaInput": {
                "localPath": "/tmp/b.mp4", "fileName": "b.mp4",
                "contentType": "video/mp4", "size": 9, "sha256": VALID_SHA256
            }}]
        });

        let error = build_upload_plan("IMAGE", "", props).expect_err("IMAGE stays image-only");
        assert!(error.to_string().contains("image/*"));
    }

    /// TEMPLATE/IMAGE 规划嵌套文件，TEMPLATE/TEXT 保持零媒体上传。
    #[test]
    fn template_image_builds_nested_upload_plan_but_text_does_not() {
        let file = file_props()["file"].clone();
        let image = build_upload_plan(
            "TEMPLATE",
            "template image",
            json!({"template":{"type":"IMAGE","file":file}}),
        )
        .expect("template image must produce a prepare plan");
        assert_eq!(image.targets, vec![UploadTarget::TemplateImage]);
        assert!(image.props["template"]["file"].get("mediaInput").is_none());

        let text = build_upload_plan(
            "TEMPLATE",
            "template text",
            json!({"template":{"type":"TEXT","text":"hello"}}),
        )
        .expect("text template must remain upload-free");
        assert!(text.media.is_empty());
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
