//! posts props 更新 outbound 命令（P2 新增）：updatePostProps（投票等 props 改写）。
//!
//! endpoint 真源 `posts.go:52`；body 真源 `posts.go:750` 匿名 struct
//! `{postId, props map[string]interface{}}`。Go 校验 `postId == "" || props == nil` → 必填。

use serde_json::{json, Map, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// POST /api/cses/posts/updatePostProps — 更新 post props（投票态等）。
/// 真源 `{postId, props}`；props 必须是非 null object（Go `props == nil` 拒）。
struct UpdatePostPropsCommand;
impl OutboundCommand for UpdatePostPropsCommand {
    fn name(&self) -> &'static str {
        "im_update_post_props"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        // props 必须是 object（Go `map[string]interface{}` 且非 nil）；边界零信任校验。
        let props = args
            .get("props")
            .filter(|v| v.is_object())
            .ok_or_else(|| ImError::Parse(format!("{}: 缺/坏 props（须为对象）", self.name())))?;
        Ok((
            "posts/updatePostProps",
            json!({ "postId": post_id, "props": props }),
        ))
    }
}
static UPDATE_POST_PROPS: UpdatePostPropsCommand = UpdatePostPropsCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_update_post_props",
        command: &UPDATE_POST_PROPS,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MessageV3 媒体消息最终 props（MV3-G01e AUDIO/VIDEO、MV3-G01g TEMPLATE/IMAGE）
//
// 线型权威 = visual 参考包 `specs/003-messagev3-gate-inventory/gates/upload/mv3-g01e`
// 与 `.../mv3-g01g` 的 `outbound-sample.json`：
//   AUDIO  props.file          = {bucket,id,uri,name,contentType,size,duration}
//   VIDEO  props.file          = {bucket,id,uri,name,contentType,size,duration,width,height}
//   TEMPLATE props.template    = {type:"IMAGE", file:{bucket,id,uri,width,height}, ...}
//
// 两条硬约束（media-host-contract §3/§6 + INV-01）：
// 1. duration / width / height 是 **Rust Host 提取** 的结果，客户端不得伪造；AUDIO 无尺寸，
//    出现 width/height 即证明该节点不是 Host 产物。
// 2. staged 绝对路径、SHA 校验上下文、预签名 URL 与 header 绝不进入 Post / 事件 / Angular。
// ─────────────────────────────────────────────────────────────────────────────

/// Host 私有 / 传输期凭据字段：一旦出现在公开媒体节点即 fail-closed（§6）。
///
/// `sha256` 在此列——参考包两份 `outbound-sample.json` 的最终 `file` 均无该键。
pub const HOST_PRIVATE_MEDIA_FIELDS: [&str; 15] = [
    "mediaInput",
    "localPath",
    "local_path",
    "localUrl",
    "local_url",
    "sha256",
    "headers",
    "url",
    "publicUrl",
    "uploadId",
    "uploadToken",
    "objectKey",
    "etag",
    "method",
    "state",
];

/// AUDIO 最终 `props.file` 的完整键集（参考包线型权威，顺序即样本顺序）。
const AUDIO_FILE_KEYS: [&str; 7] = [
    "bucket",
    "id",
    "uri",
    "name",
    "contentType",
    "size",
    "duration",
];

/// VIDEO 最终 `props.file` 的完整键集（AUDIO 键集 + Host 提取的视频轨尺寸）。
const VIDEO_FILE_KEYS: [&str; 9] = [
    "bucket",
    "id",
    "uri",
    "name",
    "contentType",
    "size",
    "duration",
    "width",
    "height",
];

/// TEMPLATE/IMAGE 内芯 `props.template.file` 的完整键集（参考包线型权威）。
const TEMPLATE_FILE_KEYS: [&str; 5] = ["bucket", "id", "uri", "width", "height"];

/// TEMPLATE 壳内芯在挂载图片时的唯一合法渲染形态（MV3-G01g「禁止回落」）。
const TEMPLATE_INNER_IMAGE: &str = "IMAGE";

/// 构造并校验媒体消息的最终 props。
///
/// 只接受 `AUDIO` / `VIDEO` / `TEMPLATE` 三类；其余类型返回 `Err`，避免被当成通用 props 规整器
/// 误用（其它类型的 props 编排真源仍在 `outbound::send_build::normalize_props`）。
///
/// 返回的 `file` / `template.file` 节点被**重建**为参考包声明的完整键集：未知键与 Host 私有键
/// 都不可能残留，泄漏面由构造方式本身消除，而不是靠事后过滤。
pub fn build_media_message_props(msg_type: &str, props: &Value) -> Result<Value, ImError> {
    let object = props
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{msg_type} props 必须是对象")))?;
    match msg_type {
        "AUDIO" | "VIDEO" => build_single_file_props(msg_type, object),
        "TEMPLATE" => build_template_props(object),
        other => Err(ImError::Parse(format!(
            "{other} 不是 MessageV3 媒体消息类型（仅 AUDIO/VIDEO/TEMPLATE）"
        ))),
    }
}

/// AUDIO / VIDEO：单 `props.file`，禁止多图 `props.files` 形态。
fn build_single_file_props(msg_type: &str, props: &Map<String, Value>) -> Result<Value, ImError> {
    if props.contains_key("files") {
        return Err(ImError::Parse(format!(
            "{msg_type} 必须使用单 props.file，不得携带 props.files"
        )));
    }
    let file = props
        .get("file")
        .ok_or_else(|| ImError::Parse(format!("{msg_type} 缺少 props.file")))?;
    let is_video = msg_type == "VIDEO";
    let mut node = stable_object_reference(file, msg_type)?;
    node.insert(
        "name".to_string(),
        json!(non_empty_str(file, "name", msg_type)?),
    );
    node.insert(
        "contentType".to_string(),
        json!(non_empty_str(file, "contentType", msg_type)?),
    );
    node.insert("size".to_string(), json!(positive_size(file, msg_type)?));
    node.insert(
        "duration".to_string(),
        json!(host_duration_seconds(file, msg_type)?),
    );
    if is_video {
        node.insert(
            "width".to_string(),
            json!(host_dimension(file, "width", msg_type)?),
        );
        node.insert(
            "height".to_string(),
            json!(host_dimension(file, "height", msg_type)?),
        );
    } else if file.get("width").is_some() || file.get("height").is_some() {
        // media-host-contract §3：Host 只为 VIDEO/IMAGE 提取尺寸。AUDIO 节点带尺寸 ⇒ 该值不是
        // Host 产物，只能来自客户端伪造，必须 fail-closed 而不是静默丢弃。
        return Err(ImError::Parse(
            "AUDIO 不得携带 width/height（Host 不为音频提取尺寸）".to_string(),
        ));
    }

    let expected: &[&str] = if is_video {
        &VIDEO_FILE_KEYS
    } else {
        &AUDIO_FILE_KEYS
    };
    debug_assert_eq!(node.len(), expected.len());
    let mut out = props.clone();
    out.insert("file".to_string(), Value::Object(node));
    Ok(Value::Object(out))
}

/// TEMPLATE/IMAGE：壳必须保留，内芯 `type` 必须是 `IMAGE`，禁止回落成 TEXT / IMAGE / FILE 消息。
fn build_template_props(props: &Map<String, Value>) -> Result<Value, ImError> {
    if props.contains_key("file") || props.contains_key("files") {
        return Err(ImError::Parse(
            "TEMPLATE 壳不得回落为顶层 props.file / props.files".to_string(),
        ));
    }
    let template = props
        .get("template")
        .and_then(Value::as_object)
        .ok_or_else(|| ImError::Parse("TEMPLATE 缺少 props.template 壳".to_string()))?;
    let inner_type = template
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("TEMPLATE 缺少 props.template.type".to_string()))?;
    let file = template
        .get("file")
        .ok_or_else(|| ImError::Parse("TEMPLATE/IMAGE 缺少 props.template.file".to_string()))?;
    if inner_type != TEMPLATE_INNER_IMAGE {
        return Err(ImError::Parse(format!(
            "TEMPLATE 挂载图片时 props.template.type 必须是 IMAGE，不得回落为 {inner_type}"
        )));
    }

    let mut node = stable_object_reference(file, "TEMPLATE/IMAGE")?;
    node.insert(
        "width".to_string(),
        json!(host_dimension(file, "width", "TEMPLATE/IMAGE")?),
    );
    node.insert(
        "height".to_string(),
        json!(host_dimension(file, "height", "TEMPLATE/IMAGE")?),
    );
    debug_assert_eq!(node.len(), TEMPLATE_FILE_KEYS.len());

    let mut inner = template.clone();
    inner.insert("type".to_string(), json!(TEMPLATE_INNER_IMAGE));
    inner.insert("file".to_string(), Value::Object(node));
    let mut out = props.clone();
    out.insert("template".to_string(), Value::Object(inner));
    Ok(Value::Object(out))
}

/// 校验 Java 归还的稳定对象引用，并返回只含 `bucket/id/uri` 的新节点。
///
/// 与 `send::upload_props::plan` 的 `is_stable_object_reference` 同规则：`uri` 必须是站内绝对
/// 路径，不得是 http(s) 预签名地址、不得含 `..` / query / fragment / 反斜杠。
fn stable_object_reference(file: &Value, label: &str) -> Result<Map<String, Value>, ImError> {
    let object = file
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{label} 媒体节点必须是对象")))?;
    if let Some(field) = HOST_PRIVATE_MEDIA_FIELDS
        .iter()
        .find(|field| object.contains_key(**field))
    {
        return Err(ImError::Parse(format!(
            "{label} 最终 props 不得携带 Host 私有字段 {field}"
        )));
    }
    let bucket = non_empty_str(file, "bucket", label)?;
    let id = non_empty_str(file, "id", label)?;
    let uri = non_empty_str(file, "uri", label)?;
    if bucket.len() > 64
        || id.len() > 255
        || !uri.starts_with('/')
        || uri.starts_with("//")
        || uri.contains("..")
        || uri.contains(['?', '#', '\\'])
    {
        return Err(ImError::Parse(format!("{label} 稳定对象引用非法")));
    }
    let mut node = Map::new();
    node.insert("bucket".to_string(), json!(bucket));
    node.insert("id".to_string(), json!(id));
    node.insert("uri".to_string(), json!(uri));
    Ok(node)
}

/// 读取必填非空字符串字段（trim 后判空）。
fn non_empty_str(node: &Value, key: &str, label: &str) -> Result<String, ImError> {
    node.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImError::Parse(format!("{label} 缺少非空 {key}")))
}

/// 字节大小：必须是正整数且不超过单文件 64 MiB 硬上限（INV-11）。
fn positive_size(node: &Value, label: &str) -> Result<u64, ImError> {
    node.get("size")
        .and_then(Value::as_u64)
        .filter(|value| (1..=(64 << 20)).contains(value))
        .ok_or_else(|| ImError::Parse(format!("{label} size 必须是 1..=67108864 的整数")))
}

/// Host 提取的时长（秒制小数，见 `media_stage.rs` 的 `round_seconds`）。
fn host_duration_seconds(node: &Value, label: &str) -> Result<f64, ImError> {
    node.get("duration")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| ImError::Parse(format!("{label} duration 必须是 Host 提取的正数秒值")))
}

/// Host 提取的像素尺寸（正整数，上界防畸形容器写出天文数字）。
fn host_dimension(node: &Value, key: &str, label: &str) -> Result<u64, ImError> {
    node.get(key)
        .and_then(Value::as_u64)
        .filter(|value| (1..=65_535).contains(value))
        .ok_or_else(|| ImError::Parse(format!("{label} {key} 必须是 Host 提取的正整数像素值")))
}

/// 递归剥除 Host 私有媒体字段，覆盖 `props.file` / `props.files[]` / `props.template.file`
/// / `props.template.files[]` 与各自的 `upload` 子对象。
///
/// `send::upload_props::persistence::sanitized_for_persistence` 只覆盖顶层 `file` / `files`，
/// TEMPLATE 壳内芯的媒体节点因此不在其射程内；本函数补齐该分支，供落库与投影前统一过闸。
pub fn strip_host_private_media_fields(props: &Value) -> Value {
    let mut sanitized = props.clone();
    if let Some(object) = sanitized.as_object_mut() {
        strip_media_container(object);
        if let Some(template) = object.get_mut("template").and_then(Value::as_object_mut) {
            strip_media_container(template);
        }
    }
    sanitized
}

/// 对一个可能同时含 `file` 与 `files[]` 的容器逐节点剥除。
fn strip_media_container(container: &mut Map<String, Value>) {
    if let Some(file) = container.get_mut("file") {
        strip_media_node(file);
    }
    if let Some(files) = container.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            strip_media_node(file);
        }
    }
}

/// 单个媒体节点及其 `upload` 子对象的字段剥除。
fn strip_media_node(node: &mut Value) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    for field in HOST_PRIVATE_MEDIA_FIELDS {
        object.remove(field);
    }
    if let Some(upload) = object.get_mut("upload").and_then(Value::as_object_mut) {
        for field in HOST_PRIVATE_MEDIA_FIELDS {
            upload.remove(field);
        }
    }
}
