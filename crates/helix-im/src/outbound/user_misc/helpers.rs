//! `user_misc.rs` 的 build 层 helper（从 user_misc.rs 外提，收口 user_misc.rs 行数基线）。
//!
//! 纯函数·零行为变更：仅承载「必填字符串数组校验」与「DoSearchRequest wire body 组装」两个
//! build-only helper，供 user_misc.rs 的命令闭包复用。边界零信任（缺/类型错 → Err，不 panic）。

use serde_json::{json, Value};

use crate::error::ImError;

/// 取必填字符串数组（非空、元素全 string）。供 `{userIds:[]}` 类命令复用。
pub(crate) fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// 组装 DoSearchRequest wire body（仅下发显式给出的字段，全 camelCase；userId 由服务端注入不传）。
/// channelId 必填（DoSearchRequest.ChannelId 非指针），其余可选。
pub(crate) fn do_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let channel_id = args.get("channel_id").and_then(Value::as_str).unwrap_or("");
    let mut b = json!({ "channelId": channel_id });
    if let Some(keyword) = args.get("keyword").and_then(Value::as_str) {
        b["keyword"] = json!(keyword);
    }
    if let Some(search_type) = args.get("search_type").and_then(Value::as_i64) {
        b["searchType"] = json!(search_type);
    }
    if let Some(team_id) = args.get("team_id").and_then(Value::as_str) {
        b["teamId"] = json!(team_id);
    }
    if let Some(is_all) = args.get("is_all").and_then(Value::as_bool) {
        b["isAll"] = json!(is_all);
    }
    // userId 是 `json:"-"`（服务端 session 注入）；前端传了也禁止下发（防契约漂移）。
    if args.get("user_id").is_some() {
        return Err(ImError::Parse(format!(
            "{cmd}: user_id 由服务端 session 注入，禁前端下发（DoSearchRequest.UserId json:\"-\"）"
        )));
    }
    Ok(b)
}
