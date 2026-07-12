//! 多目标转发 HTTP 接收结果投影；accepted 明确不等于 delivered。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};

pub(crate) fn emit(req_id: &str, body: &Value) -> Effect {
    let targets_in = body
        .get("data")
        .and_then(|data| data.get("targets"))
        .and_then(Value::as_array);
    let targets: Vec<Value> = targets_in
        .into_iter()
        .flatten()
        .filter_map(|target| {
            let channel_id = target.get("channelId")?.as_str()?.trim();
            if channel_id.is_empty() {
                return None;
            }
            let accepted = target.get("status").and_then(Value::as_str) == Some("accepted");
            Some(json!({
                "channelId": channel_id,
                "acceptanceStatus": if accepted { "accepted" } else { "failed" },
                "deliveryStatus": if accepted { "pending" } else { "not-applicable" },
                "error": target.get("error").and_then(Value::as_str),
            }))
        })
        .collect();
    let accepted_count = targets
        .iter()
        .filter(|target| target["acceptanceStatus"] == "accepted")
        .count();
    let failed_count = targets.len().saturating_sub(accepted_count);
    let batch_status = match (accepted_count, failed_count) {
        (0, _) => "failed",
        (_, 0) => "success",
        _ => "partial",
    };
    emit_value(json!({
        "reqId": req_id,
        "batchStatus": batch_status,
        "acceptedCount": accepted_count,
        "failedCount": failed_count,
        "deliveryAuthority": "ws-post",
        "targets": targets,
    }))
}

pub(crate) fn emit_error(req_id: &str, error: &str) -> Effect {
    emit_value(json!({
        "reqId": req_id,
        "batchStatus": "failed",
        "acceptedCount": 0,
        "failedCount": 0,
        "deliveryAuthority": "ws-post",
        "targets": [],
        "error": error,
    }))
}

pub(crate) fn emit_target_error(req_id: &str, channel_ids: &[String], error: &str) -> Effect {
    let targets: Vec<Value> = channel_ids
        .iter()
        .map(|channel_id| {
            json!({
                "channelId": channel_id,
                "acceptanceStatus": "failed",
                "deliveryStatus": "not-applicable",
                "error": error,
            })
        })
        .collect();
    emit_value(json!({
        "reqId": req_id,
        "batchStatus": "failed",
        "acceptedCount": 0,
        "failedCount": targets.len(),
        "deliveryAuthority": "ws-post",
        "targets": targets,
        "error": error,
    }))
}

fn emit_value(data: Value) -> Effect {
    let payload = json!({ "event": "im:posts:batch-result", "data": data });
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("static batch result must serialize"),
        )),
    }
}
