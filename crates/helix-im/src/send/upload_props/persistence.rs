use super::TemporaryId;
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

/// 生成可落库/进入 posts.create 的 props 副本。
///
/// PUT URL 与签名 headers 只留在进程内的 PendingSend，任何持久化入口都必须使用本副本。
pub fn sanitized_for_persistence(props: &Value) -> Value {
    let mut sanitized = props.clone();
    if let Some(images) = sanitized.get_mut("images").and_then(Value::as_array_mut) {
        for image in images {
            strip_write_credentials(image);
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
        node.remove("userId");
        node.remove("teamId");
        node.remove("etag");
    }
    if let Some(upload) = node.get_mut("upload").and_then(Value::as_object_mut) {
        upload.remove("url");
        upload.remove("headers");
        upload.remove("userId");
        upload.remove("teamId");
        upload.remove("etag");
    }
}

/// 上传 props 的统一持久化闸门；解析异常同样 fail-closed 为无凭据空对象。
pub fn props_persist_op(temporary_id: &TemporaryId, props_json: String) -> StorageOp {
    let props_json = serde_json::from_str::<Value>(&props_json)
        .map(|props| sanitized_for_persistence(&props))
        .and_then(|props| serde_json::to_string(&props))
        .unwrap_or_else(|_| "{}".to_string());
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
