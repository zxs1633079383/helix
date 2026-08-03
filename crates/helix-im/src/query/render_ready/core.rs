//! S6 消息族 render-ready 行整形（issue #55·C013 纯渲染壳）。
//!
//! 把**读族**消息行（首屏 `Scan(message)` 的 DB snake 列 / 上拉 `postContext` 的 wire camel Post）
//! 整形成前端可 **1:1 绑定**的终态行——前端 `applyMessagesQueryResult` / `applyOlderLoaded` 退化为
//! 纯绑定（不再壳内 snake 抽取 / 归一 readBits / dedup）。整形/去重/归一**下沉 helix**。
//!
//! 统一字段集（camelCase·与前端 `MessageRow` 对齐）：
//!   `msgId`（server id·缺退 temporaryId） · `temporaryId` · `channelId` · `eventSeq`
//!   （优先行字段，回退持久化 `props.channel_event_seq`，均缺则 null）
//!   · `sendStatus`("sent"·读族=已落库) · `readBits`(string·number→string 归一) · `text`
//!   · `type`(空→"TEXT") · `revoked`(bool·DB `revoke` 0/1) · `createAt`(i64|null) · `userId`。
//!
//! 同源整形（DB snake 与 wire camel 两形态用同一 getter 容错读），保证两条读路径吐**同一 render-ready 形态**。

use serde_json::{json, Value};
use std::collections::HashSet;

/// 取首个命中且非空的字符串列（snake / camel 容错）。全缺 → `""`。
fn get_str<'a>(row: &'a Value, keys: &[&str]) -> &'a str {
    for k in keys {
        if let Some(s) = row.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s;
            }
        }
    }
    ""
}

/// `read_bits` / `readBits` 归一为 string（number→to_string·缺/异常→""）。
fn read_bits_str(row: &Value) -> String {
    match row.get("read_bits").or_else(|| row.get("readBits")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// DB `revoke`（0/1/bool）→ 撤回 bool（缺/异常→false）。wire Post 无此列 → false。
fn revoke_bool(row: &Value) -> bool {
    match row.get("revoke") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().map(|x| x != 0).unwrap_or(false),
        _ => false,
    }
}

fn props_value(row: &Value) -> Value {
    match row.get("props") {
        Some(Value::Object(_)) => row["props"].clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn event_seq_value(row: &Value, props: &Value) -> Value {
    row.get("event_seq")
        .or_else(|| row.get("eventSeq"))
        .and_then(Value::as_u64)
        .or_else(|| props.get("channel_event_seq").and_then(Value::as_u64))
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn upload_progress_percent(row: &Value) -> u8 {
    row.get("upload_progress_percent")
        .or_else(|| row.get("progressPercent"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        })
        .unwrap_or_default()
        .min(100) as u8
}

fn topic_value(row: &Value) -> Value {
    match row.get("topic") {
        Some(Value::Object(_)) => row["topic"].clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn json_value(row: &Value, keys: &[&str], fallback: Value) -> Value {
    keys.iter()
        .find_map(|key| row.get(*key))
        .map(|value| match value {
            Value::String(raw) => serde_json::from_str::<Value>(raw)
                .ok()
                .filter(|parsed| !parsed.is_null())
                .unwrap_or_else(|| fallback.clone()),
            Value::Null => fallback.clone(),
            other => other.clone(),
        })
        .unwrap_or(fallback)
}

/// Normalize identity snapshot aliases without inventing a display name.
pub(crate) fn normalize_user_snapshot(value: &Value) -> Value {
    normalize_object_aliases(
        value,
        &[
            ("user_id", "userId"),
            ("user_name", "userName"),
            ("nick_name", "nickName"),
            ("dept_name", "deptName"),
            ("org_name", "orgName"),
            ("team_id", "teamId"),
            ("company_id", "companyId"),
            ("company_name", "companyName"),
            ("dept_id", "deptId"),
            ("org_id", "orgId"),
        ],
    )
}

/// Normalize reply snapshot aliases while preserving the stored identity values.
pub(crate) fn normalize_replied_message(value: &Value) -> Value {
    let normalized = normalize_object_aliases(
        value,
        &[
            ("replied_user_id", "repliedUserId"),
            ("replied_user_name", "repliedUserName"),
            ("is_revoke", "isRevoke"),
            ("simple_message", "simpleMessage"),
            ("reply_id", "replyId"),
            ("reply_root_id", "replyRootId"),
            ("reply_first_level_id", "replyFirstLevelId"),
        ],
    );
    let Value::Object(mut object) = normalized else {
        return normalized;
    };
    if !object.contains_key("message") {
        if let Some(message) = object
            .get("text")
            .or_else(|| object.get("simpleMessage"))
            .cloned()
        {
            object.insert("message".to_string(), message);
        }
    }
    Value::Object(object)
}

/// Convert selected snake_case object keys to their render-ready camelCase aliases.
fn normalize_object_aliases(value: &Value, aliases: &[(&str, &str)]) -> Value {
    let Value::Object(mut object) = value.clone() else {
        return value.clone();
    };
    for (snake, camel) in aliases {
        if !object.contains_key(*camel) {
            if let Some(value) = object.remove(*snake) {
                object.insert((*camel).to_string(), value);
            }
        } else {
            object.remove(*snake);
        }
    }
    Value::Object(object)
}

fn json_container_len(value: &Value) -> i64 {
    match value {
        Value::Array(items) => items.len() as i64,
        Value::Object(items) => items.len() as i64,
        _ => 0,
    }
}

fn template_received_bool(row: &Value) -> bool {
    if let Some(b) = row.get("templateReceived").and_then(|v| v.as_bool()) {
        return b;
    }
    props_value(row)
        .get("template")
        .and_then(|v| v.get("userIds").or_else(|| v.get("user_ids")))
        .and_then(|v| v.as_array())
        .map(|ids| !ids.is_empty())
        .unwrap_or(false)
}

fn array_len_at(value: &Value, first: &str, second: &str) -> usize {
    value
        .get(first)
        .or_else(|| value.get(second))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn interaction_summary(props: &Value) -> (Option<String>, usize, usize) {
    let mut emojis = Vec::new();
    let mut reaction_count = 0;
    match props.get("quickReply") {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(emoji) = item.get("emoji").and_then(Value::as_str) {
                    if !emoji.is_empty() {
                        emojis.push(emoji);
                    }
                }
                reaction_count += array_len_at(item, "userIds", "user_ids");
            }
        }
        Some(Value::Object(items)) => {
            for (emoji, user_ids) in items {
                if !emoji.is_empty() {
                    emojis.push(emoji.as_str());
                }
                reaction_count += user_ids.as_array().map(Vec::len).unwrap_or(0);
            }
        }
        _ => {}
    }
    let reactions = (!emojis.is_empty()).then(|| emojis.join(","));
    let template_count = props
        .get("template")
        .map(|template| array_len_at(template, "userIds", "user_ids"))
        .unwrap_or(0);
    (reactions, reaction_count, template_count)
}

fn template_reader_ids(props: &Value) -> Vec<&str> {
    props
        .get("template")
        .and_then(|template| template.get("userIds").or_else(|| template.get("user_ids")))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| !id.is_empty())
        .collect()
}

/// 单行整形：DB snake 列 / wire camel Post → render-ready 终态行（camelCase·前端 1:1 绑定）。
pub(crate) fn shape_row(row: &Value, viewer_user_id: &str) -> Value {
    let temporary_id = get_str(row, &["temporary_id", "temporaryId"]);
    let server_id = get_str(row, &["id"]);
    let channel_id = get_str(row, &["channel_id", "channelId"]);
    let text = get_str(row, &["message", "text"]);
    let simple_message_raw = get_str(row, &["simple_message", "simpleMessage"]);
    let simple_message = if simple_message_raw.is_empty() {
        text
    } else {
        simple_message_raw
    };
    let msg_type_raw = get_str(row, &["type"]);
    let msg_type = if msg_type_raw.is_empty() {
        "TEXT"
    } else {
        msg_type_raw
    };
    let direct_user_id = get_str(row, &["user_id", "userId"]);
    let user_snapshot = normalize_user_snapshot(&json_value(
        row,
        &["user_snapshot", "userSnapshot"],
        json!({}),
    ));
    let snapshot_user_id = get_str(&user_snapshot, &["user_id", "userId"]);
    let creator_user_id = get_str(row, &["create_by", "createBy"]);
    let user_id = if !direct_user_id.is_empty() {
        direct_user_id
    } else if !snapshot_user_id.is_empty() {
        snapshot_user_id
    } else {
        creator_user_id
    };
    let create_at = row
        .get("create_at")
        .or_else(|| row.get("createAt"))
        .and_then(|v| v.as_i64());
    // msgId：server id 优先（已对账历史行多有 server id），缺退 temporaryId（与 send 链锚一致）。
    let msg_id = if server_id.is_empty() {
        temporary_id
    } else {
        server_id
    };
    let send_status = match get_str(row, &["send_status", "sendStatus"]) {
        "unsend" | "failed" => "failed",
        "sending" => "sending",
        _ => "sent",
    };
    let props = props_value(row);
    let event_seq = event_seq_value(row, &props);
    let forward_detail = super::forward::detail(msg_type, &props);
    let expedite_map = row
        .get("expedite_map")
        .or_else(|| row.get("expediteMap"))
        .cloned()
        .and_then(|value| match value {
            Value::String(raw) => serde_json::from_str(&raw).ok(),
            other => Some(other),
        })
        .unwrap_or_else(|| json!({}));
    let is_self = !viewer_user_id.is_empty() && user_id == viewer_user_id;
    let has_server_id = !server_id.is_empty();
    let (reactions, reaction_count, template_confirmed_count) = interaction_summary(&props);
    let urgent = super::urgent::project(&expedite_map, viewer_user_id, has_server_id);
    let urgent_fields = json!({
        "urgent": urgent.required_count > 0,
        "urgentTargetCount": urgent.required_count,
        "urgentConfirmedCount": urgent.confirmed_ids.len(),
        "urgentRequesterId": urgent.requester_id,
        "urgentTargetIds": urgent.target_ids,
        "urgentConfirmedIds": urgent.confirmed_ids,
        "urgentRequiredCount": urgent.required_count,
        "urgentState": urgent.state,
        "canConfirmUrgent": urgent.can_confirm,
    });
    let viewers = json_value(row, &["viewers"], json!([]));
    let mentions = json_value(row, &["mentions"], json!([]));
    let replied_message = normalize_replied_message(&json_value(
        row,
        &["replied_message", "repliedMessage"],
        Value::Null,
    ));
    let reply_messages = json_value(row, &["reply_messages", "replyMessages"], json!([]));
    let reply_count = row
        .get("reply_count")
        .or_else(|| row.get("replyCount"))
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .max(json_container_len(&reply_messages));
    let template_received = template_received_bool(row);
    let template_reader_ids = template_reader_ids(&props);
    let template_confirmation = json!({
        "templateReceived": template_received,
        "confirmedCount": template_confirmed_count,
        "readerIds": template_reader_ids,
    });
    let mut data = json!({
        "id": msg_id,
        "msgId": msg_id,
        "temporaryId": temporary_id,
        "channelId": channel_id,
        "eventSeq": event_seq,
        "sendStatus": send_status,
        "progressPercent": upload_progress_percent(row),
        "readBits": read_bits_str(row),
        "message": text,
        "text": text,
        "simpleMessage": simple_message,
        "type": msg_type,
        "props": props,
        "forwardDetail": forward_detail,
        "topic": topic_value(row),
        "revoke": revoke_bool(row),
        "revoked": revoke_bool(row),
        "templateReceived": template_received,
        "templateConfirmedCount": template_confirmed_count,
        "readerIds": template_reader_ids,
        "templateConfirmation": template_confirmation,
        "serverId": if has_server_id { Value::String(server_id.to_string()) } else { Value::Null },
        "isSelf": is_self,
        "reactions": reactions,
        "reactionCount": reaction_count,
        "expediteMap": expedite_map,
        "replyId": get_str(row, &["reply_id", "replyId"]),
        "replyRootId": get_str(row, &["reply_root_id", "replyRootId"]),
        "replyFirstLevelId": get_str(row, &["reply_first_level_id", "replyFirstLevelId"]),
        "repliedMessage": replied_message,
        "replyMessages": reply_messages,
        "replyCount": reply_count,
        "createAt": create_at,
        "createdAt": create_at,
        "userId": user_id,
        "userSnapshot": user_snapshot,
        "viewers": viewers,
        "mentions": mentions,
        "teamId": get_str(row, &["team_id", "teamId"]),
        "snapshotId": get_str(row, &["snapshot_id", "snapshotId"]),
    });
    if let (Value::Object(data), Value::Object(urgent_fields)) = (&mut data, urgent_fields) {
        data.extend(urgent_fields);
    }
    data
}

/// 历史消息行批整形：读族行数组（DB snake 或 wire camel）→ render-ready 行数组。
///
/// **批内按 `msgId` 去重·保序**（dedup 下沉 helix·前端不再 findIndex 全表扫去重）；非对象 / 无锚行跳过。
/// 上拉路径 emit 前已 `createAt` 升序，本函数保序（不重排），保证「更早在前」prepend 语义。
pub fn shape_message_rows(rows: &Value) -> Value {
    shape_message_rows_for_viewer(rows, "")
}

pub fn shape_message_rows_for_viewer(rows: &Value, viewer_user_id: &str) -> Value {
    let arr = match rows.as_array() {
        Some(a) => a,
        None => return json!([]),
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Value> = Vec::with_capacity(arr.len());
    for row in arr {
        if !row.is_object() {
            continue;
        }
        let shaped = shape_row(row, viewer_user_id);
        let id = shaped["msgId"].as_str().unwrap_or("").to_string();
        if id.is_empty() || !seen.insert(id) {
            continue;
        }
        out.push(shaped);
    }
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A persisted snake row keeps its sender identity and reply payload in the render contract.
    #[test]
    fn shape_row_preserves_message_identity_and_reply_fields() {
        let row = json!({
            "id": "w7qox39odbydp8arbfdo4e756r",
            "temporary_id": "helix_tmp_0000016",
            "channel_id": "nzwwqjqskjd43drywr1gc6cw7c",
            "event_seq": 29,
            "user_id": "444",
            "user_snapshot": "{\"user_id\":\"444\",\"user_name\":\"破坏者\",\"dept_name\":\"生产部\",\"org_name\":\"科研人员\"}",
            "type": "TEXT",
            "message": "回复消息",
            "simple_message": "回复消息",
            "props": "{}",
            "viewers": "[\"all\"]",
            "mentions": "[]",
            "replied_message": "{\"replied_user_id\":\"444\",\"replied_user_name\":\"破坏者\",\"message\":\"被回复消息\",\"is_revoke\":false,\"type\":\"TEXT\",\"props\":{},\"viewers\":[\"all\"]}",
            "create_at": 1785720540703i64
        });

        let shaped = shape_row(&row, "viewer");
        assert_eq!(shaped["message"], "回复消息");
        assert_eq!(shaped["text"], "回复消息");
        assert_eq!(shaped["simpleMessage"], "回复消息");
        assert_eq!(shaped["userId"], "444");
        assert_eq!(shaped["userSnapshot"]["userId"], "444");
        assert_eq!(shaped["userSnapshot"]["userName"], "破坏者");
        assert_eq!(shaped["viewers"][0], "all");
        assert_eq!(shaped["mentions"], json!([]));
        assert_eq!(shaped["repliedMessage"]["repliedUserId"], "444");
        assert_eq!(shaped["repliedMessage"]["repliedUserName"], "破坏者");
        assert_eq!(shaped["repliedMessage"]["message"], "被回复消息");
    }
}
