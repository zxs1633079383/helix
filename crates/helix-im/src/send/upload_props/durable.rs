use super::{
    FailedMediaOp, MediaInput, PendingMediaOp, PendingMediaPrepare, PendingMediaPut, PreparedMedia,
    UploadTarget,
};
use crate::state::{ChannelId, TemporaryId};
use crate::ImError;
use helix_core::effect::{BatchDeleteSpec, SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

const TABLE: &str = "pending_media";

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
    };
    let prepared_text =
        |read: fn(&PreparedMedia) -> &String| prepared.map(read).cloned().unwrap_or_default();

    let size = i64::try_from(input.size)
        .map_err(|_| ImError::Parse("pending_media size exceeds SQLite INTEGER".to_string()))?;
    Ok(StorageOp::BatchUpsert(UpsertSpec {
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
                SqlValue::Text(prepared_text(|value| &value.upload_id)),
            ),
            (
                "file_id".to_string(),
                SqlValue::Text(prepared_text(|value| &value.file_id)),
            ),
            (
                "method".to_string(),
                SqlValue::Text(prepared_text(|value| &value.method)),
            ),
            (
                "upload_url".to_string(),
                // 预签名写地址是短期 secret，任何 durable stage 都只落空值。
                SqlValue::Text(String::new()),
            ),
            (
                "public_url".to_string(),
                SqlValue::Text(prepared_text(|value| &value.public_url)),
            ),
            (
                "object_key".to_string(),
                SqlValue::Text(prepared_text(|value| &value.object_key)),
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

pub fn upsert_many(operations: &[PendingMediaOp]) -> Result<StorageOp, ImError> {
    let mut rows = Vec::with_capacity(operations.len());
    for operation in operations {
        let StorageOp::BatchUpsert(spec) = upsert_op(operation)? else {
            unreachable!("durable media upsert always builds BatchUpsert")
        };
        rows.extend(spec.rows);
    }
    Ok(StorageOp::BatchUpsert(UpsertSpec {
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

pub fn decode_scan(bytes: &[u8]) -> Result<Vec<FailedMediaOp>, ImError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ImError::Parse(format!("pending_media scan: {error}")))?;
    rows.iter().map(decode_row).collect()
}

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
        .filter(|value| *value > 0)
        .ok_or_else(|| ImError::Parse("pending_media invalid size".to_string()))?;
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
        "prepare" => Ok(FailedMediaOp::Prepare(prepare)),
        // PUT 未确认完成时，旧预签名可能已经失效；重启只能回到 Prepare。
        "put" => Ok(FailedMediaOp::Prepare(prepare)),
        "complete" => {
            let pending = PendingMediaPut {
                temporary_id,
                channel_id,
                target,
                input,
                prepared: PreparedMedia {
                    upload_id: required("upload_id")?,
                    file_id: required("file_id")?,
                    method: required("method")?,
                    url: String::new(),
                    public_url: required("public_url")?,
                    object_key: required("object_key")?,
                    headers: Vec::new(),
                },
            };
            Ok(FailedMediaOp::Complete(pending))
        }
        _ => Err(ImError::Parse("pending_media invalid stage".to_string())),
    }
}

fn target_key(target: &UploadTarget) -> String {
    match target {
        UploadTarget::File => "file".to_string(),
        UploadTarget::RichImage { index } => format!("rich:{index}"),
    }
}

fn parse_target(value: &str) -> Result<UploadTarget, ImError> {
    if value == "file" {
        return Ok(UploadTarget::File);
    }
    value
        .strip_prefix("rich:")
        .and_then(|index| index.parse::<usize>().ok())
        .map(|index| UploadTarget::RichImage { index })
        .ok_or_else(|| ImError::Parse("pending_media invalid target_key".to_string()))
}
