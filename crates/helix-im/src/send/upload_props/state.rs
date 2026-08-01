use super::{PendingMediaPut, UploadSuccess, UploadTarget};
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
    target.apply_upload_state(props, "success", Some(FULL_PERCENT))
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
    // 失败冻结当前绝对进度：Angular 重试卡片需要显示「卡在多少」，不得回退成 0。
    target.apply_upload_state(props, "failed", None)
}

/// 将 PUT 2xx 后的服务端 bucket/uri 与本地媒体元数据写入公开消息引用。
pub fn mark_media_complete(props: &mut Value, pending: &PendingMediaPut) -> Result<(), ImError> {
    let node = target_node_mut(props, &pending.target)?;
    let object = node
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("media target must be object".to_string()))?;
    for field in [
        "mediaInput",
        "localPath",
        "localUrl",
        "local_url",
        "local_path",
        "objectKey",
        "uploadId",
        "fileId",
        "headers",
        "userId",
        "teamId",
        "etag",
        "method",
        "state",
        "publicUrl",
        "url",
        // Host 暂存句柄是账号绑定的 opaque token：解析后不得随最终引用出站或落 UI 投影。
        "mediaHandle",
        "media_handle",
    ] {
        object.remove(field);
    }
    object.insert("bucket".to_string(), json!(&pending.prepared.bucket));
    object.insert("id".to_string(), json!(&pending.prepared.file_id));
    object.insert("name".to_string(), json!(&pending.input.file_name));
    object.insert("uri".to_string(), json!(&pending.prepared.uri));
    object.insert(
        "contentType".to_string(),
        json!(&pending.input.content_type),
    );
    object.insert("size".to_string(), json!(pending.input.size));
    object.insert("sha256".to_string(), json!(&pending.input.sha256));
    object.insert("upload".to_string(), json!({"status": "completed"}));
    // 单目标收口仍是 O(1)；消息级加权投影只在「某个附件真正完成」这一 n 次边界重算一次，
    // 不挂在高频 progress tick 上。
    pending
        .target
        .apply_upload_state(props, "completed", Some(FULL_PERCENT))?;
    let weighted = UploadTarget::message_upload_progress(props)?;
    props
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("props must be object".to_string()))?
        .insert("uploadProgress".to_string(), json!(weighted));
    Ok(())
}

pub fn mark_media_stage(
    props: &mut Value,
    target: &UploadTarget,
    status: &str,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    node.as_object_mut()
        .ok_or_else(|| ImError::Parse("media target must be object".to_string()))?
        .insert("upload".to_string(), json!({"status": status}));
    target.apply_upload_state(props, status, None)
}

/// 单文件绝对进度上限；消息级加权投影同样以百分比收口。
const FULL_PERCENT: u64 = 100;

/// 媒体节点尚未开始上传时的绝对进度状态。
pub(super) const PENDING_STATUS: &str = "pending";

/// 登记媒体目标时写入初始绝对进度（INV-01：乐观 Sending 首帧即自带 per-file 进度，
/// Angular 不得自算）。只碰目标节点自身，规划阶段仍是每个目标 O(1)。
pub(super) fn init_upload_state(object: &mut serde_json::Map<String, Value>) {
    object.insert("uploadStatus".to_string(), json!(PENDING_STATUS));
    object.insert("uploadProgress".to_string(), json!(0u64));
}

impl UploadTarget {
    /// 按 O(1) 直接定位单个媒体节点并写入绝对进度与状态。
    ///
    /// 该函数**只读写目标节点**：既不遍历兄弟节点，也不触发消息级重算。多图场景下
    /// 单文件进度推进因此恒为 O(1)，不会退化成整表扫描（T076）。
    /// `percent = None` 表示保留已有绝对进度（失败态冻结），仅在首次登记时补 0。
    pub fn apply_upload_state(
        &self,
        props: &mut Value,
        status: &str,
        percent: Option<u64>,
    ) -> Result<(), ImError> {
        let node = target_node_mut(props, self)?;
        let object = node
            .as_object_mut()
            .ok_or_else(|| ImError::Parse("media target must be object".to_string()))?;
        object.insert("uploadStatus".to_string(), json!(status));
        match percent {
            Some(percent) => {
                object.insert(
                    "uploadProgress".to_string(),
                    json!(percent.min(FULL_PERCENT)),
                );
            }
            None => {
                object
                    .entry("uploadProgress".to_string())
                    .or_insert_with(|| json!(0));
            }
        }
        Ok(())
    }

    /// 消息级上传进度：按**总字节加权**的 O(n) 单趟投影。
    ///
    /// 权重取每个媒体节点的 `size`，进度取该节点的绝对 `uploadProgress`；
    /// 已经是稳定对象引用（bucket+id+uri 且无上传态）的节点按 100 计入。
    /// 与「按文件数取平均」严格区分：1024B@100% + 2048B@0% 必须是 33 而不是 50。
    pub fn message_upload_progress(props: &Value) -> Result<u64, ImError> {
        let mut weighted = 0u128;
        let mut total = 0u128;
        let mut accumulate = |node: &Value| -> Result<(), ImError> {
            let object = node
                .as_object()
                .ok_or_else(|| ImError::Parse("media target must be object".to_string()))?;
            let size = object.get("size").and_then(Value::as_u64).unwrap_or(0);
            let percent = match object.get("uploadProgress").and_then(Value::as_u64) {
                Some(percent) => percent.min(FULL_PERCENT),
                None if is_settled_reference(object) => FULL_PERCENT,
                None => 0,
            };
            weighted += u128::from(size) * u128::from(percent);
            total += u128::from(size);
            Ok(())
        };
        let template_file = props
            .get("template")
            .and_then(Value::as_object)
            .and_then(|template| template.get("file"));
        match (props.get("files"), props.get("file"), template_file) {
            (Some(files), _, _) => {
                let files = files
                    .as_array()
                    .ok_or_else(|| ImError::Parse("props.files must be array".to_string()))?;
                for node in files {
                    accumulate(node)?;
                }
            }
            (None, Some(file), _) => accumulate(file)?,
            (None, None, Some(file)) => accumulate(file)?,
            (None, None, None) => return Ok(0),
        }
        if total == 0 {
            return Ok(0);
        }
        Ok((weighted / total) as u64)
    }
}

/// 无本地上传态、且已带完整服务端引用的节点：转发/复用场景零上传即 100%。
fn is_settled_reference(object: &serde_json::Map<String, Value>) -> bool {
    ["bucket", "id", "uri"]
        .iter()
        .all(|field| object.get(*field).and_then(Value::as_str).is_some())
}

fn target_node_mut<'a>(
    props: &'a mut Value,
    target: &UploadTarget,
) -> Result<&'a mut Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get_mut("files")
            .and_then(Value::as_array_mut)
            .and_then(|files| files.get_mut(*index))
            .ok_or_else(|| ImError::Parse("missing rich image target".to_string())),
        UploadTarget::File => props
            .get_mut("file")
            .ok_or_else(|| ImError::Parse("missing file target".to_string())),
        UploadTarget::TemplateImage => props
            .get_mut("template")
            .and_then(Value::as_object_mut)
            .and_then(|template| template.get_mut("file"))
            .ok_or_else(|| ImError::Parse("missing template image target".to_string())),
    }
}
