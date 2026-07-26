//! Mobile older-context canonical request parsing and Go wire body construction.

use serde_json::{json, Value};

use super::{LoadOlderState, LOAD_OLDER_CONTEXT, TARGET_DEFAULT, TARGET_MAX, TARGET_MIN};
use crate::error::ImError;
use crate::state::ChannelId;

/// 解析 `im_load_older_context` payload：业务字段外仅接受可选 transport `req_id`。
pub fn parse_request(payload: &[u8]) -> Result<LoadOlderState, ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} payload: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} payload 必须为 object")))?;
    if let Some(unknown) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "channel_id" | "anchor_post_id" | "before" | "req_id"
        )
    }) {
        return Err(ImError::Parse(format!(
            "{LOAD_OLDER_CONTEXT} 未知字段: {unknown}"
        )));
    }

    let channel_str = object
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} 缺 channel_id")))?;
    let channel_id = ChannelId::from_str(channel_str)
        .ok_or_else(|| ImError::Parse(format!("非法 channel_id: {channel_str}")))?;
    let anchor_post_id = object
        .get("anchor_post_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} 缺 anchor_post_id")))?
        .to_string();
    let target = parse_before(object.get("before"))?;
    let request_id = parse_request_id(object.get("req_id"))?;
    let mut state = LoadOlderState::new(channel_id, anchor_post_id, target);
    state.attach_request_id(request_id);

    Ok(state)
}

/// Validates the optional host correlation without promoting it into business input.
fn parse_request_id(value: Option<&Value>) -> Result<Option<String>, ImError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let request_id = value
        .as_str()
        .filter(|request_id| !request_id.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} req_id 必须为非空字符串")))?;
    Ok(Some(request_id.to_string()))
}

/// Validates the bounded page size used by each postContext round.
fn parse_before(value: Option<&Value>) -> Result<u32, ImError> {
    let Some(value) = value else {
        return Ok(TARGET_DEFAULT);
    };
    let before = value
        .as_i64()
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} before 必须为整数")))?;
    if !(i64::from(TARGET_MIN)..=i64::from(TARGET_MAX)).contains(&before) {
        return Err(ImError::Parse(format!(
            "{LOAD_OLDER_CONTEXT} before 必须在 1..=100"
        )));
    }
    Ok(before as u32)
}

/// 构造一轮 Go `postContext` 的 `(endpoint_path, wire_body)`。
pub fn build_post_context_body(state: &LoadOlderState) -> (&'static str, Value) {
    (
        "posts/postContext",
        json!({ "postId": state.pivot_id(), "before": state.target() }),
    )
}
