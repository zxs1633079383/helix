//! 建群候选用户 HTTP 响应整形成 UI 可直接绑定的稳定投影。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};

/// 把候选目录 HTTP body 整形成冻结的 `im:user:candidates` 投影。
pub fn emit(req_id: &str, body: &Value) -> Effect {
    let users = body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(shape_user)
        .collect::<Vec<_>>();
    emit_value(json!({ "reqId": req_id, "users": users }))
}

pub(crate) fn emit_error(req_id: &str, error: &str) -> Effect {
    emit_value(json!({ "reqId": req_id, "users": [], "error": error }))
}

fn shape_user(user: &Value) -> Option<Value> {
    let user_id = nonempty(user, "id")?;
    let username = string(user, "username");
    let nickname = string(user, "nickname");
    let display_name = if !nickname.is_empty() {
        nickname.clone()
    } else if !username.is_empty() {
        username.clone()
    } else {
        user_id.clone()
    };
    Some(json!({
        "userId": user_id,
        "username": username,
        "nickname": nickname,
        "displayName": display_name,
        "deptId": string(user, "deptId"),
        "deptName": string(user, "deptName"),
        "orgId": string(user, "orgId"),
        "orgName": string(user, "orgName"),
    }))
}

fn nonempty(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn emit_value(data: Value) -> Effect {
    let observe = data.get("error").is_some_and(Value::is_string);
    let payload = json!({ "event": "im:user:candidates", "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("static candidates projection must serialize"),
    );
    Effect::Emit {
        event: if observe {
            DomainEventBytes::observed(bytes)
        } else {
            DomainEventBytes(bytes)
        },
    }
}
