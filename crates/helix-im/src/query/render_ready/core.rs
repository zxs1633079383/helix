//! S6 消息族 render-ready 行整形（issue #55·C013 纯渲染壳）。
//!
//! 把**读族**消息行（首屏 `Scan(message)` 的 DB snake 列 / 上拉 `postContext` 的 wire camel Post）
//! 整形成前端可 **1:1 绑定**的终态行——前端 `applyMessagesQueryResult` / `applyOlderLoaded` 退化为
//! 纯绑定（不再壳内 snake 抽取 / 归一 readBits / dedup）。整形/去重/归一**下沉 helix**。
//!
//! 统一字段集（camelCase·与前端 `MessageRow` 对齐）：
//!   `msgId`（server id·缺退 temporaryId） · `temporaryId` · `channelId` · `eventSeq`(null·读族无列)
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

/// 单行整形：DB snake 列 / wire camel Post → render-ready 终态行（camelCase·前端 1:1 绑定）。
fn shape_row(row: &Value) -> Value {
    let temporary_id = get_str(row, &["temporary_id", "temporaryId"]);
    let server_id = get_str(row, &["id"]);
    let channel_id = get_str(row, &["channel_id", "channelId"]);
    let text = get_str(row, &["message"]);
    let msg_type_raw = get_str(row, &["type"]);
    let msg_type = if msg_type_raw.is_empty() {
        "TEXT"
    } else {
        msg_type_raw
    };
    let user_id = get_str(row, &["user_id", "userId"]);
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
    json!({
        "id": msg_id,
        "msgId": msg_id,
        "temporaryId": temporary_id,
        "channelId": channel_id,
        "eventSeq": Value::Null,
        "sendStatus": "sent",
        "readBits": read_bits_str(row),
        "text": text,
        "type": msg_type,
        "props": props_value(row),
        "revoke": revoke_bool(row),
        "revoked": revoke_bool(row),
        "templateReceived": template_received_bool(row),
        "createAt": create_at,
        "createdAt": create_at,
        "userId": user_id,
    })
}

/// 历史消息行批整形：读族行数组（DB snake 或 wire camel）→ render-ready 行数组。
///
/// **批内按 `msgId` 去重·保序**（dedup 下沉 helix·前端不再 findIndex 全表扫去重）；非对象 / 无锚行跳过。
/// 上拉路径 emit 前已 `createAt` 升序，本函数保序（不重排），保证「更早在前」prepend 语义。
pub fn shape_message_rows(rows: &Value) -> Value {
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
        let shaped = shape_row(row);
        let id = shaped["msgId"].as_str().unwrap_or("").to_string();
        if id.is_empty() || !seen.insert(id) {
            continue;
        }
        out.push(shaped);
    }
    Value::Array(out)
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
