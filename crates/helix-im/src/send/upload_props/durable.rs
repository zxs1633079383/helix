use super::{
    FailedMediaOp, MediaInput, MultipartSession, PendingMediaOp, PendingMediaPrepare,
    PendingMediaPut, PreparedMedia, UploadTarget, MAX_MEDIA_SIZE_BYTES,
};
use crate::state::{ChannelId, TemporaryId};
use crate::ImError;
use helix_core::effect::{BatchDeleteSpec, SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

const TABLE: &str = "pending_media";

/// 将媒体步骤写入兼容旧 schema 的 durable journal，且不持久化预签名 URL。
pub fn upsert_op(operation: &PendingMediaOp) -> Result<StorageOp, ImError> {
    let (temporary_id, channel_id, target, input, prepared, stage) = match operation {
        PendingMediaOp::Prepare(pending) => (
            &pending.temporary_id,
            pending.channel_id,
            &pending.target,
            &pending.input,
            None,
            "prepare",
        ),
        PendingMediaOp::Put(pending) => (
            &pending.temporary_id,
            pending.channel_id,
            &pending.target,
            &pending.input,
            Some(&pending.prepared),
            "put",
        ),
        PendingMediaOp::Complete(pending) => (
            &pending.temporary_id,
            pending.channel_id,
            &pending.target,
            &pending.input,
            Some(&pending.prepared),
            "complete",
        ),
        PendingMediaOp::MultipartInit(pending) => (
            &pending.temporary_id,
            pending.channel_id,
            &pending.target,
            &pending.input,
            None,
            "multipart_init",
        ),
        PendingMediaOp::MultipartParts(session) => (
            &session.temporary_id,
            session.channel_id,
            &session.target,
            &session.input,
            None,
            "multipart_parts",
        ),
        PendingMediaOp::MultipartSign(pending) => (
            &pending.session.temporary_id,
            pending.session.channel_id,
            &pending.session.target,
            &pending.session.input,
            None,
            "multipart_sign",
        ),
        PendingMediaOp::MultipartPut(pending) => (
            &pending.session.temporary_id,
            pending.session.channel_id,
            &pending.session.target,
            &pending.session.input,
            None,
            "multipart_parts",
        ),
        PendingMediaOp::MultipartComplete(session) => (
            &session.temporary_id,
            session.channel_id,
            &session.target,
            &session.input,
            None,
            "multipart_complete",
        ),
    };
    let prepared_text =
        |read: fn(&PreparedMedia) -> &String| prepared.map(read).cloned().unwrap_or_default();
    let (multipart_file_id, multipart_object_key, multipart_uri) = match operation {
        PendingMediaOp::MultipartParts(session) | PendingMediaOp::MultipartComplete(session) => (
            session.file_id.clone(),
            session.object_key.clone(),
            session.uri.clone(),
        ),
        PendingMediaOp::MultipartSign(pending) => (
            pending.session.file_id.clone(),
            pending.session.object_key.clone(),
            pending.session.uri.clone(),
        ),
        PendingMediaOp::MultipartPut(pending) => (
            pending.session.file_id.clone(),
            pending.session.object_key.clone(),
            pending.session.uri.clone(),
        ),
        _ => (String::new(), String::new(), String::new()),
    };
    let is_multipart = matches!(
        operation,
        PendingMediaOp::MultipartInit(_)
            | PendingMediaOp::MultipartParts(_)
            | PendingMediaOp::MultipartSign(_)
            | PendingMediaOp::MultipartPut(_)
            | PendingMediaOp::MultipartComplete(_)
    );

    let size = i64::try_from(input.size)
        .map_err(|_| ImError::Parse("pending_media size exceeds SQLite INTEGER".to_string()))?;
    Ok(StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: TABLE,
        rows: vec![vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text(temporary_id.0.clone()),
            ),
            ("target_key".to_string(), SqlValue::Text(target_key(target))),
            ("stage".to_string(), SqlValue::Text(stage.to_string())),
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.as_str().to_string()),
            ),
            (
                "local_path".to_string(),
                SqlValue::Text(input.local_path.clone()),
            ),
            (
                "file_name".to_string(),
                SqlValue::Text(input.file_name.clone()),
            ),
            (
                "content_type".to_string(),
                SqlValue::Text(input.content_type.clone()),
            ),
            ("size".to_string(), SqlValue::Integer(size)),
            ("sha256".to_string(), SqlValue::Text(input.sha256.clone())),
            (
                "upload_id".to_string(),
                // Multipart 的 uploadId 属于规范化 session；旧列仍保持空，避免
                // 过去把 upload_id 当 fileId 的歧义在恢复时重新出现。
                SqlValue::Text(if is_multipart {
                    String::new()
                } else {
                    prepared_text(|value| &value.file_id)
                }),
            ),
            (
                "file_id".to_string(),
                SqlValue::Text(if is_multipart {
                    multipart_file_id
                } else {
                    prepared_text(|value| &value.file_id)
                }),
            ),
            (
                "method".to_string(),
                SqlValue::Text(if is_multipart {
                    "MULTIPART".to_string()
                } else {
                    prepared_text(|value| &value.method)
                }),
            ),
            (
                "upload_url".to_string(),
                // 预签名写地址是短期 secret，任何 durable stage 都只落空值。
                SqlValue::Text(String::new()),
            ),
            (
                "public_url".to_string(),
                // 兼容既有 SQLite 列名；新媒体合同在此保存 bucket 内 uri。
                SqlValue::Text(if is_multipart {
                    multipart_uri
                } else {
                    prepared_text(|value| &value.uri)
                }),
            ),
            (
                "object_key".to_string(),
                SqlValue::Text(if is_multipart {
                    multipart_object_key
                } else {
                    prepared_text(|value| &value.object_key)
                }),
            ),
            (
                "headers_json".to_string(),
                // 签名头与 upload_url 同寿命，不跨失败或进程重启复用。
                SqlValue::Text("[]".to_string()),
            ),
        ]],
        conflict_key: None,
        exclude_from_update: Vec::new(),
    }))
}

/// Persist the bounded media-operation batch under existing sparse identities.
pub fn upsert_many(operations: &[PendingMediaOp]) -> Result<StorageOp, ImError> {
    let mut rows = Vec::with_capacity(operations.len());
    for operation in operations {
        let StorageOp::BatchUpsert(spec) = upsert_op(operation)? else {
            unreachable!("durable media upsert always builds BatchUpsert")
        };
        rows.extend(spec.rows);
    }
    Ok(StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: TABLE,
        rows,
        conflict_key: None,
        exclude_from_update: Vec::new(),
    }))
}

pub fn delete_op(temporary_id: &TemporaryId, target: &UploadTarget) -> StorageOp {
    StorageOp::BatchDelete(BatchDeleteSpec {
        table: TABLE,
        scope_col: "temporary_id",
        scope_val: SqlValue::Text(temporary_id.0.clone()),
        key_col: "target_key",
        key_vals: vec![SqlValue::Text(target_key(target))],
    })
}

/// Normalized multipart session journal. The signed URL and headers are
/// intentionally absent; only durable identity and input facts survive a
/// restart.
pub fn multipart_session_upsert_op(session: &MultipartSession) -> StorageOp {
    let size = i64::try_from(session.input.size).unwrap_or(i64::MAX);
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "media_upload_session",
        rows: vec![vec![
            (
                "session_id".to_string(),
                SqlValue::Text(session.client_upload_id.clone()),
            ),
            (
                "temporary_id".to_string(),
                SqlValue::Text(session.temporary_id.0.clone()),
            ),
            (
                "target_key".to_string(),
                SqlValue::Text(target_key(&session.target)),
            ),
            (
                "channel_id".to_string(),
                SqlValue::Text(session.channel_id.as_str().to_string()),
            ),
            (
                "client_upload_id".to_string(),
                SqlValue::Text(session.client_upload_id.clone()),
            ),
            ("bucket".to_string(), SqlValue::Text(session.bucket.clone())),
            (
                "content_type".to_string(),
                SqlValue::Text(session.input.content_type.clone()),
            ),
            ("size".to_string(), SqlValue::Integer(size)),
            (
                "sha256".to_string(),
                SqlValue::Text(session.input.sha256.clone()),
            ),
            (
                "file_name".to_string(),
                SqlValue::Text(session.input.file_name.clone()),
            ),
            (
                "local_path".to_string(),
                SqlValue::Text(session.input.local_path.clone()),
            ),
            (
                "upload_id".to_string(),
                SqlValue::Text(session.upload_id.clone()),
            ),
            (
                "file_id".to_string(),
                SqlValue::Text(session.file_id.clone()),
            ),
            (
                "object_key".to_string(),
                SqlValue::Text(session.object_key.clone()),
            ),
            ("uri".to_string(), SqlValue::Text(session.uri.clone())),
            (
                "part_size".to_string(),
                SqlValue::Integer(session.part_size as i64),
            ),
            ("status".to_string(), SqlValue::Text(session.status.clone())),
        ]],
        conflict_key: None,
        exclude_from_update: Vec::new(),
    })
}

/// Seed the normalized session row before Java init. It is intentionally an
/// incomplete session: `upload_id`/file identity are filled only after the
/// matching init response is validated.
pub fn multipart_prepare_session_upsert_op(pending: &PendingMediaPrepare) -> StorageOp {
    let session = MultipartSession {
        temporary_id: pending.temporary_id.clone(),
        channel_id: pending.channel_id,
        target: pending.target.clone(),
        input: pending.input.clone(),
        client_upload_id: super::client_upload_id(&pending.temporary_id.0, &pending.target),
        upload_id: String::new(),
        file_id: String::new(),
        bucket: pending.target.bucket().to_string(),
        object_key: String::new(),
        uri: String::new(),
        part_size: super::MULTIPART_PART_SIZE_BYTES,
        status: "preparing".to_string(),
        parts: Vec::new(),
    };
    multipart_session_upsert_op(&session)
}

/// Persist only confirmed remote parts. A signed part is never written here.
pub fn multipart_parts_upsert_op(session: &MultipartSession) -> Option<StorageOp> {
    if session.parts.is_empty() {
        return None;
    }
    Some(StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "media_upload_part",
        rows: session
            .parts
            .iter()
            .map(|part| {
                vec![
                    (
                        "session_id".to_string(),
                        SqlValue::Text(session.client_upload_id.clone()),
                    ),
                    (
                        "part_number".to_string(),
                        SqlValue::Integer(i64::from(part.part_number)),
                    ),
                    ("size".to_string(), SqlValue::Integer(part.size as i64)),
                    ("etag".to_string(), SqlValue::Text(part.etag.clone())),
                    (
                        "status".to_string(),
                        SqlValue::Text("completed".to_string()),
                    ),
                ]
            })
            .collect(),
        conflict_key: None,
        exclude_from_update: Vec::new(),
    }))
}

/// Recovery reconciliation replaces the bounded remote list. Delete the
/// session's previous sparse rows before the optional batch upsert so a part
/// missing from Java/OSS cannot survive locally as a false checkpoint.
pub fn multipart_parts_delete_op(session: &MultipartSession) -> StorageOp {
    StorageOp::BatchDelete(BatchDeleteSpec {
        table: "media_upload_part",
        scope_col: "session_id",
        scope_val: SqlValue::Text(session.client_upload_id.clone()),
        key_col: "part_number",
        key_vals: (1..=session.input.part_count())
            .map(|part| SqlValue::Integer(i64::from(part)))
            .collect(),
    })
}

/// Normal path checkpoint: one confirmed ETag becomes one sparse row. The
/// complete remote parts list is the only recovery boundary that may batch rows.
pub fn multipart_part_upsert_op(
    session: &MultipartSession,
    part: &super::MultipartPart,
) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "media_upload_part",
        rows: vec![vec![
            (
                "session_id".to_string(),
                SqlValue::Text(session.client_upload_id.clone()),
            ),
            (
                "part_number".to_string(),
                SqlValue::Integer(i64::from(part.part_number)),
            ),
            ("size".to_string(), SqlValue::Integer(part.size as i64)),
            ("etag".to_string(), SqlValue::Text(part.etag.clone())),
            (
                "status".to_string(),
                SqlValue::Text("completed".to_string()),
            ),
        ]],
        conflict_key: None,
        exclude_from_update: Vec::new(),
    })
}

pub fn multipart_delete_ops(session: &MultipartSession) -> Vec<StorageOp> {
    let mut ops = vec![StorageOp::BatchDelete(BatchDeleteSpec {
        table: "media_upload_session",
        scope_col: "session_id",
        scope_val: SqlValue::Text(session.client_upload_id.clone()),
        key_col: "session_id",
        key_vals: vec![SqlValue::Text(session.client_upload_id.clone())],
    })];
    ops.push(StorageOp::BatchDelete(BatchDeleteSpec {
        table: "media_upload_part",
        scope_col: "session_id",
        scope_val: SqlValue::Text(session.client_upload_id.clone()),
        key_col: "part_number",
        key_vals: (1..=session.input.part_count())
            .map(|part| SqlValue::Integer(i64::from(part)))
            .collect(),
    }));
    ops
}

pub fn decode_scan(bytes: &[u8]) -> Result<Vec<FailedMediaOp>, ImError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ImError::Parse(format!("pending_media scan: {error}")))?;
    rows.iter().map(decode_row).collect()
}

// 从兼容旧 schema 的一行恢复可安全重试的媒体步骤。
fn decode_row(row: &Value) -> Result<FailedMediaOp, ImError> {
    let required = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ImError::Parse(format!("pending_media invalid {key}")))
    };
    let temporary_id = TemporaryId(required("temporary_id")?);
    let channel_id = ChannelId::from_str(required("channel_id")?.as_str())
        .ok_or_else(|| ImError::Parse("pending_media invalid channel_id".to_string()))?;
    let target = parse_target(required("target_key")?.as_str())?;
    let size = row
        .get("size")
        .and_then(Value::as_u64)
        .filter(|value| (1..=MAX_MEDIA_SIZE_BYTES).contains(value))
        .ok_or_else(|| {
            ImError::Parse(format!(
                "pending_media size must be in 1..={MAX_MEDIA_SIZE_BYTES}"
            ))
        })?;
    let input = MediaInput {
        local_path: required("local_path")?,
        file_name: required("file_name")?,
        content_type: required("content_type")?,
        size,
        sha256: required("sha256")?,
    };
    let prepare = PendingMediaPrepare {
        temporary_id: temporary_id.clone(),
        channel_id,
        target: target.clone(),
        input: input.clone(),
    };
    match required("stage")?.as_str() {
        "prepare" => {
            if input.uses_multipart() {
                Ok(FailedMediaOp::Multipart(prepare))
            } else {
                Ok(FailedMediaOp::Prepare(prepare))
            }
        }
        // PUT 未确认完成时，旧预签名可能已经失效；重启只能回到 Prepare。
        "put" => {
            if input.uses_multipart() {
                Ok(FailedMediaOp::Multipart(prepare))
            } else {
                Ok(FailedMediaOp::Prepare(prepare))
            }
        }
        stage if stage.starts_with("multipart_") => Ok(FailedMediaOp::Multipart(prepare)),
        "complete" => {
            let bucket = target.bucket().to_string();
            let pending = PendingMediaPut {
                temporary_id,
                channel_id,
                target,
                input,
                prepared: PreparedMedia {
                    file_id: required("file_id")?,
                    bucket,
                    method: required("method")?,
                    url: String::new(),
                    uri: required("public_url")?,
                    object_key: required("object_key")?,
                    headers: Vec::new(),
                },
            };
            Ok(FailedMediaOp::Complete(pending))
        }
        _ => Err(ImError::Parse("pending_media invalid stage".to_string())),
    }
}

/// 将上传目标编码为兼容旧图片 journal 的稳定键。
fn target_key(target: &UploadTarget) -> String {
    match target {
        UploadTarget::File => "file".to_string(),
        UploadTarget::RichImage { index } => format!("rich:{index}"),
        UploadTarget::RichVideo { index } => format!("rich-video:{index}"),
        UploadTarget::TemplateImage => "template:image".to_string(),
    }
}

/// 从 durable journal 恢复图片、视频或单文件上传目标。
fn parse_target(value: &str) -> Result<UploadTarget, ImError> {
    if value == "file" {
        return Ok(UploadTarget::File);
    }
    if value == "template:image" {
        return Ok(UploadTarget::TemplateImage);
    }
    if let Some(index) = value
        .strip_prefix("rich-video:")
        .and_then(|index| index.parse::<usize>().ok())
    {
        return Ok(UploadTarget::RichVideo { index });
    }
    value
        .strip_prefix("rich:")
        .and_then(|index| index.parse::<usize>().ok())
        .map(|index| UploadTarget::RichImage { index })
        .ok_or_else(|| ImError::Parse("pending_media invalid target_key".to_string()))
}
