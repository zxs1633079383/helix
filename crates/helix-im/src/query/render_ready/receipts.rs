//! 单条消息回执人员投影；只消费 Go 的 readerIds/readers，绝不解释 readBits。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};

pub(crate) fn emit(req_id: &str, body: &Value) -> Effect {
    let receipts: Vec<Value> = body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let post_id = item.get("postId")?.as_str()?.trim();
            if post_id.is_empty() {
                return None;
            }
            let reader_ids: Vec<&str> = item
                .get("readerIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
                .collect();
            let readers: Vec<Value> = reader_ids
                .iter()
                .map(|user_id| json!({ "userId": user_id }))
                .collect();
            Some(json!({
                "postId": post_id,
                "readerCount": reader_ids.len(),
                "readerIds": reader_ids,
                "readers": readers,
            }))
        })
        .collect();
    emit_value(json!({ "reqId": req_id, "receipts": receipts }))
}

pub(crate) fn emit_error(req_id: &str, error: &str) -> Effect {
    emit_value(json!({ "reqId": req_id, "receipts": [], "error": error }))
}

fn emit_value(data: Value) -> Effect {
    let payload = json!({ "event": "im:post:readers", "data": data });
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("static receipt projection must serialize"),
        )),
    }
}
