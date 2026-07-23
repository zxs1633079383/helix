use super::TemporaryId;
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

/// 生成可落库/进入 posts.create 的 props 副本。
///
/// PUT URL 与签名 headers 只留在进程内的 PendingSend，任何持久化入口都必须使用本副本。
pub fn sanitized_for_persistence(props: &Value) -> Value {
    let mut sanitized = props.clone();
    if let Some(files) = sanitized.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            strip_write_credentials(file);
        }
    }
    if let Some(file) = sanitized.get_mut("file") {
        strip_write_credentials(file);
    }
    sanitized
}

fn strip_write_credentials(node: &mut Value) {
    if let Some(node) = node.as_object_mut() {
        node.remove("mediaInput");
        node.remove("localPath");
        for field in [
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
        ] {
            node.remove(field);
        }
    }
    if let Some(upload) = node.get_mut("upload").and_then(Value::as_object_mut) {
        upload.remove("url");
        upload.remove("headers");
        upload.remove("userId");
        upload.remove("teamId");
        upload.remove("etag");
        upload.remove("objectKey");
        upload.remove("uploadId");
        upload.remove("fileId");
        upload.remove("method");
        upload.remove("state");
        upload.remove("publicUrl");
    }
}

/// 上传 props 的统一持久化闸门；解析异常同样 fail-closed 为无凭据空对象。
pub fn props_persist_op(temporary_id: &TemporaryId, props_json: String) -> StorageOp {
    let props_json = sanitized_props_json(&props_json);
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

/// 媒体发送阶段的单行终态写：props、发送状态与可选进度必须由同一条 upsert 提交，
/// 避免失败帧看到新 props 却仍读取旧 `send_status` / 旧百分比。
pub fn media_send_state_persist_op(
    temporary_id: &TemporaryId,
    props_json: String,
    send_status: &str,
    progress_percent: Option<u8>,
) -> StorageOp {
    let mut row = vec![
        (
            "temporary_id".to_string(),
            SqlValue::Text(temporary_id.0.clone()),
        ),
        (
            "props".to_string(),
            SqlValue::Text(sanitized_props_json(&props_json)),
        ),
        (
            "send_status".to_string(),
            SqlValue::Text(send_status.to_string()),
        ),
    ];
    if let Some(progress_percent) = progress_percent {
        row.push((
            "upload_progress_percent".to_string(),
            SqlValue::Integer(i64::from(progress_percent)),
        ));
    }
    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![row],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
}

fn sanitized_props_json(props_json: &str) -> String {
    serde_json::from_str::<Value>(props_json)
        .map(|props| sanitized_for_persistence(&props))
        .and_then(|props| serde_json::to_string(&props))
        .unwrap_or_else(|_| "{}".to_string())
}
