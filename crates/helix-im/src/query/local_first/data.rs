//! local-first 的纯数据规则：coverage 判定、远端解码、合并与缓存 op。

use std::cmp::Ordering;
use std::collections::BTreeMap;

use helix_core::effect::StorageOp;
use helix_core::tick::ReplyBytes;
use serde_json::Value;

use crate::error::ImError;
use crate::state::{ChannelId, Seq};

use super::{LocalReadCoverage, LocalStoreMode, MessageQueryRequest, RecentMessageCoverage};

pub(super) fn classify_local_read(
    mode: LocalStoreMode,
    request: &MessageQueryRequest,
    local_rows_desc: &[Value],
    proof: Option<&RecentMessageCoverage>,
    has_known_gap: bool,
) -> LocalReadCoverage {
    if mode == LocalStoreMode::Disabled {
        return LocalReadCoverage::Unsupported;
    }
    if local_rows_desc.is_empty() {
        return LocalReadCoverage::Miss;
    }
    if has_known_gap {
        return LocalReadCoverage::Partial;
    }
    let Some(proof) = proof else {
        return LocalReadCoverage::Partial;
    };
    let local_keys: Vec<String> = local_rows_desc.iter().filter_map(message_key).collect();
    if local_keys.len() != local_rows_desc.len() {
        return LocalReadCoverage::Partial;
    }
    let requested = request.limit as usize;

    let exact_window = if requested <= proof.remote_keys_desc.len() {
        local_keys.len() >= requested
            && local_keys[..requested] == proof.remote_keys_desc[..requested]
    } else if proof.remote_exhausted {
        local_keys == proof.remote_keys_desc
    } else {
        false
    };
    if exact_window {
        LocalReadCoverage::Complete
    } else {
        LocalReadCoverage::Partial
    }
}

pub(crate) fn parse_local_rows(bytes: &[u8]) -> Result<Vec<Value>, ImError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| ImError::Parse(format!("message scan reply: {error}")))?;
    let rows = value
        .as_array()
        .ok_or_else(|| ImError::Parse("message scan reply must be an array".to_string()))?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err(ImError::Parse(
            "message scan reply rows must be objects".to_string(),
        ));
    }
    Ok(rows.clone())
}

/// 校验 host HTTP 信封、HTTP status、Go CommonRes status，再提取合法 posts 数组。
pub(super) fn parse_latest_posts_reply(reply: &ReplyBytes) -> Result<Vec<Value>, ImError> {
    let envelope: Value = serde_json::from_slice(reply.0.as_ref())
        .map_err(|error| ImError::Parse(format!("getLatestPost envelope: {error}")))?;
    let status = envelope
        .get("status")
        .and_then(Value::as_u64)
        .ok_or_else(|| ImError::Parse("getLatestPost envelope missing numeric status".into()))?;
    if !(200..300).contains(&status) {
        return Err(ImError::Parse(format!(
            "getLatestPost HTTP status {status}"
        )));
    }
    let raw = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())?;
    parse_latest_posts_body(&raw)
}

pub(crate) fn parse_latest_posts_body(raw_body: &[u8]) -> Result<Vec<Value>, ImError> {
    let root: Value = serde_json::from_slice(raw_body)
        .map_err(|error| ImError::Parse(format!("getLatestPost body: {error}")))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse("getLatestPost body missing string status".into()))?;
    if !status.eq_ignore_ascii_case("SUCCESS") {
        let detail = root
            .get("message")
            .or_else(|| root.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("unknown backend error");
        return Err(ImError::Parse(format!(
            "getLatestPost backend status {status}: {detail}"
        )));
    }

    let candidate = [
        root.pointer("/data/posts"),
        root.pointer("/data/items"),
        root.pointer("/data/list"),
        root.get("data"),
    ]
    .into_iter()
    .flatten()
    .find(|value| value.is_array());
    if root.pointer("/data/posts").is_some_and(Value::is_null) {
        return Ok(Vec::new());
    }
    let rows = candidate
        .and_then(Value::as_array)
        .ok_or_else(|| ImError::Parse("getLatestPost response missing posts array".into()))?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err(ImError::Parse(
            "getLatestPost posts must be objects".to_string(),
        ));
    }
    Ok(rows.clone())
}

#[cfg(test)]
mod latest_posts_tests {
    use super::parse_latest_posts_body;

    /// Go V3 空 Timeline 的 `posts:null` 表示合法零消息，而不是协议缺失。
    #[test]
    fn latest_posts_accepts_null_posts_as_empty_timeline() {
        let rows = parse_latest_posts_body(
            br#"{"status":"SUCCESS","data":{"posts":null,"hasOlder":false,"hasNewer":false}}"#,
        )
        .expect("empty V3 timeline must parse");
        assert!(rows.is_empty());
    }

    /// SUCCESS 信封仍必须显式携带 posts 字段，不能把任意对象误判为空 Timeline。
    #[test]
    fn latest_posts_rejects_missing_posts_contract() {
        let error = parse_latest_posts_body(
            br#"{"status":"SUCCESS","data":{"hasOlder":false,"hasNewer":false}}"#,
        )
        .expect_err("missing posts must fail closed");
        assert!(error.to_string().contains("missing posts array"));
    }
}

pub(crate) fn post_event_seq(post: &Value) -> Option<u64> {
    post.get("eventSeq")
        .or_else(|| post.get("event_seq"))
        .and_then(Value::as_u64)
        .or_else(|| {
            let props = post.get("props")?;
            if let Some(raw) = props.as_str() {
                serde_json::from_str::<Value>(raw)
                    .ok()?
                    .get("channel_event_seq")
                    .and_then(Value::as_u64)
            } else {
                props.get("channel_event_seq").and_then(Value::as_u64)
            }
        })
}

/// 将 Go wire Post 统一转换为可见 render-ready row 与同源 message upsert。
pub(crate) fn visible_remote_rows_and_cache_ops(
    channel_id: ChannelId,
    posts: Vec<Value>,
    local_rows_desc: &[Value],
    auth_user_id: &str,
) -> Result<(Vec<Value>, Vec<StorageOp>), ImError> {
    let local_sequences = local_event_sequences(local_rows_desc);
    let mut rows = Vec::with_capacity(posts.len());
    let mut ops = Vec::with_capacity(posts.len());
    for mut post in posts {
        let mut fields = crate::ws::parser::extract_post_fields(&post);
        let source_presence = fields.present_fields;
        if post_event_seq(&post).is_none() {
            let local_sequence = local_sequences
                .server
                .get(fields.id.as_str())
                .or_else(|| local_sequences.temporary.get(fields.temporary_id.as_str()))
                .copied();
            if let Some(sequence) = local_sequence {
                preserve_local_event_sequence(&mut post, sequence);
                fields = crate::ws::parser::extract_post_fields(&post);
                // 本地 cursor 注入的 props 不是 HTTP 明确字段，不能改变 readback 覆盖语义。
                fields.present_fields = source_presence;
            }
        }
        if fields.channel_id.is_empty() {
            fields.channel_id = channel_id.as_str().to_string();
        } else if fields.channel_id != channel_id.as_str() {
            return Err(ImError::Parse(format!(
                "getLatestPost channel mismatch: requested {}, got {}",
                channel_id.as_str(),
                fields.channel_id
            )));
        }
        let update =
            crate::channel_write::post_updates_from_fields(channel_id, &fields, auth_user_id);
        if !update.visible {
            continue;
        }
        if fields.id.is_empty() && fields.temporary_id.is_empty() {
            return Err(ImError::Parse(
                "getLatestPost post missing id and temporaryId".to_string(),
            ));
        }
        if let Some(local) = matching_local_message_row(&post, &fields, local_rows_desc) {
            overlay_missing_remote_fields(&mut post, local, &fields);
        }
        normalize_remote_row(&mut post, channel_id, &fields);
        let seq = post_event_seq(&post).unwrap_or(0);
        let msg_id = if fields.id.is_empty() {
            fields.temporary_id.clone()
        } else {
            fields.id.clone()
        };
        let event = crate::sync_session::EventEnvelope::new(
            channel_id,
            Seq(seq),
            crate::sync_session::EventKind::PostUpsert,
            fields,
        )
        .with_msg_id(Some(msg_id));
        ops.push(crate::channel::event_to_readback_upsert_op(&event));
        rows.push(post);
    }
    Ok((rows, ops))
}

/// Finds the local row by server id first, then temporary id, matching the message identity contract.
fn matching_local_message_row<'a>(
    remote: &Value,
    fields: &crate::sync_session::PostFields,
    local_rows: &'a [Value],
) -> Option<&'a Value> {
    let remote_server = if !fields.id.is_empty() {
        fields.id.as_str()
    } else {
        server_id(remote)
    };
    if !remote_server.is_empty() {
        if let Some(row) = local_rows
            .iter()
            .find(|row| server_id(row) == remote_server)
        {
            return Some(row);
        }
    }
    let remote_temporary = if !fields.temporary_id.is_empty() {
        fields.temporary_id.as_str()
    } else {
        temporary_id(remote)
    };
    (!remote_temporary.is_empty()).then(|| {
        local_rows
            .iter()
            .find(|row| temporary_id(row) == remote_temporary)
    })?
}

/// Overlays only absent remote fields so explicit remote empty values retain authority.
fn overlay_missing_remote_fields(
    remote: &mut Value,
    local: &Value,
    fields: &crate::sync_session::PostFields,
) {
    let Some(remote_object) = remote.as_object_mut() else {
        return;
    };
    let fields_to_overlay = [
        (
            crate::sync_session::POST_FIELD_ID,
            "id",
            &["id", "msgId", "postId", "post_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_CHANNEL_ID,
            "channelId",
            &["channelId", "channel_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_USER_ID,
            "userId",
            &["userId", "user_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_TYPE,
            "type",
            &["type"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_MESSAGE,
            "message",
            &["message"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_SIMPLE_MESSAGE,
            "simpleMessage",
            &["simpleMessage", "simple_message"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_PROPS,
            "props",
            &["props"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_USER_SNAPSHOT,
            "userSnapshot",
            &["userSnapshot", "user_snapshot"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_VIEWERS,
            "viewers",
            &["viewers"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_MENTIONS,
            "mentions",
            &["mentions"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_EXPEDITE_MAP,
            "expediteMap",
            &["expediteMap", "expedite_map"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_TOPIC,
            "topic",
            &["topic"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_QUICK_REPLY,
            "quickReply",
            &["quickReply", "quick_reply"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_REPLY_ID,
            "replyId",
            &["replyId", "reply_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_REPLY_ROOT_ID,
            "replyRootId",
            &["replyRootId", "reply_root_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_REPLY_FIRST_LEVEL_ID,
            "replyFirstLevelId",
            &["replyFirstLevelId", "reply_first_level_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_REPLIED_MESSAGE,
            "repliedMessage",
            &["repliedMessage", "replied_message"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_REPLY_MESSAGES,
            "replyMessages",
            &["replyMessages", "reply_messages"][..],
            true,
        ),
        (
            crate::sync_session::POST_FIELD_REPLY_COUNT,
            "replyCount",
            &["replyCount", "reply_count"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_READ_BITS,
            "readBits",
            &["readBits", "read_bits"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_SNAPSHOT_ID,
            "snapshotId",
            &["snapshotId", "snapshot_id"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_CREATE_AT,
            "createAt",
            &["createAt", "createdAt", "create_at"][..],
            false,
        ),
        (
            crate::sync_session::POST_FIELD_UPDATE_AT,
            "updateAt",
            &["updateAt", "update_at"][..],
            false,
        ),
    ];
    for (field, remote_key, local_keys, parse_json) in fields_to_overlay {
        if fields.has_field(field) {
            continue;
        }
        let Some(value) = find_local_field(local, local_keys) else {
            continue;
        };
        remote_object.insert(remote_key.to_string(), local_wire_value(value, parse_json));
    }
}

/// Looks up a local row field across camelCase and SQLite snake_case names.
fn find_local_field<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| row.get(*key))
}

/// Converts SQLite JSON text columns back to JSON values for the UI projection.
fn local_wire_value(value: &Value, parse_json: bool) -> Value {
    if !parse_json {
        return value.clone();
    }
    value
        .as_str()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| value.clone())
}

#[derive(Default)]
struct LocalEventSequences {
    server: BTreeMap<String, u64>,
    temporary: BTreeMap<String, u64>,
}

fn local_event_sequences(local_rows_desc: &[Value]) -> LocalEventSequences {
    let mut sequences = LocalEventSequences::default();
    for row in local_rows_desc {
        let Some(sequence) = post_event_seq(row) else {
            continue;
        };
        let server = server_id(row);
        if !server.is_empty() {
            sequences
                .server
                .entry(server.to_string())
                .and_modify(|stored| *stored = (*stored).max(sequence))
                .or_insert(sequence);
        }
        let temporary = temporary_id(row);
        if !temporary.is_empty() {
            sequences
                .temporary
                .entry(temporary.to_string())
                .and_modify(|stored| *stored = (*stored).max(sequence))
                .or_insert(sequence);
        }
    }
    sequences
}

fn preserve_local_event_sequence(post: &mut Value, sequence: u64) {
    let Some(object) = post.as_object_mut() else {
        return;
    };
    object.insert("eventSeq".to_string(), Value::from(sequence));
    let props = object
        .remove("props")
        .unwrap_or_else(|| serde_json::json!({}));
    let mut props = match props {
        Value::Object(props) => props,
        Value::String(raw) => serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        _ => serde_json::Map::new(),
    };
    props
        .entry("channel_event_seq".to_string())
        .or_insert_with(|| Value::from(sequence));
    object.insert("props".to_string(), Value::Object(props));
}

/// Fills only transport identity defaults while preserving readback field presence semantics.
fn normalize_remote_row(
    post: &mut Value,
    channel_id: ChannelId,
    fields: &crate::sync_session::PostFields,
) {
    let Some(object) = post.as_object_mut() else {
        return;
    };
    if !fields.id.is_empty() {
        object
            .entry("id")
            .or_insert_with(|| Value::String(fields.id.clone()));
    }
    if !fields.temporary_id.is_empty() {
        object
            .entry("temporaryId")
            .or_insert_with(|| Value::String(fields.temporary_id.clone()));
    }
    object
        .entry("channelId")
        .or_insert_with(|| Value::String(channel_id.as_str().to_string()));
    object
        .entry("createAt")
        .or_insert_with(|| Value::from(fields.create_at));
    object
        .entry("updateAt")
        .or_insert_with(|| Value::from(fields.update_at));
    if fields.has_field(crate::sync_session::POST_FIELD_PROPS) && !fields.props.is_empty() {
        object.insert(
            "props".to_string(),
            serde_json::from_str(&fields.props).unwrap_or(Value::Null),
        );
    }
}

pub(super) fn merge_recent_rows(
    local_rows_desc: Vec<Value>,
    remote_rows_desc: Vec<Value>,
    remote_exhausted: bool,
    limit: usize,
) -> Vec<Value> {
    let capacity = local_rows_desc.len() + remote_rows_desc.len();
    let oldest_remote_create_at = remote_rows_desc.iter().map(create_at).min();
    let newest_remote_create_at = remote_rows_desc.iter().map(create_at).max();
    // 低频查询、上限 500；remote 先入使权威行覆盖同 server id/temporaryId 的本地副本。
    // exhausted 时远端只证明其当前快照到头；本地仍保留比远端最新行更新的已提交事实，
    // 避免读副本延迟把刚由 sync 持久化的消息误判为 stale。
    // 未 exhausted 时，本地 server 行也只能补在权威最近窗口之前，不能挤回窗口内。
    let eligible_local = local_rows_desc.into_iter().filter(|row| {
        if server_id(row).is_empty() {
            return !temporary_id(row).is_empty();
        }
        if remote_exhausted {
            return newest_remote_create_at
                .map(|newest| create_at(row) > newest)
                .unwrap_or(false);
        }
        oldest_remote_create_at
            .map(|oldest| create_at(row) < oldest)
            .unwrap_or(false)
    });
    let mut merged = Vec::with_capacity(capacity);
    for row in remote_rows_desc.into_iter().chain(eligible_local) {
        if !merged
            .iter()
            .any(|existing| rows_share_message_identity(existing, &row))
        {
            merged.push(row);
        }
    }
    sort_recent_rows_desc(&mut merged);
    merged.truncate(limit);
    merged
}

pub(super) fn stale_local_server_rows_delete_op(
    channel_id: ChannelId,
    local_rows_desc: &[Value],
    remote_rows_desc: &[Value],
) -> Option<StorageOp> {
    let newest_remote_create_at = remote_rows_desc.iter().map(create_at).max();
    let key_vals = local_rows_desc
        .iter()
        .filter(|local| {
            !server_id(local).is_empty()
                && newest_remote_create_at
                    .map(|newest| create_at(local) <= newest)
                    .unwrap_or(true)
                && !remote_rows_desc
                    .iter()
                    .any(|remote| rows_share_message_identity(local, remote))
        })
        .filter_map(|row| {
            let key = temporary_id(row);
            if key.is_empty() {
                let fallback = server_id(row);
                (!fallback.is_empty()).then(|| helix_core::effect::SqlValue::Text(fallback.into()))
            } else {
                Some(helix_core::effect::SqlValue::Text(key.into()))
            }
        })
        .collect::<Vec<_>>();
    if key_vals.is_empty() {
        return None;
    }
    Some(StorageOp::BatchDelete(
        helix_core::effect::BatchDeleteSpec {
            table: "message",
            scope_col: "channel_id",
            scope_val: helix_core::effect::SqlValue::Text(channel_id.as_str().to_string()),
            key_col: "temporary_id",
            key_vals,
        },
    ))
}

pub(super) fn dedup_recent_rows(rows: &mut Vec<Value>) {
    let mut unique: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if !unique
            .iter()
            .any(|existing| rows_share_message_identity(existing, &row))
        {
            unique.push(row);
        }
    }
    *rows = unique;
}

fn rows_share_message_identity(left: &Value, right: &Value) -> bool {
    let left_server = server_id(left);
    let right_server = server_id(right);
    if !left_server.is_empty() && left_server == right_server {
        return true;
    }
    let left_temporary = temporary_id(left);
    let right_temporary = temporary_id(right);
    !left_temporary.is_empty() && left_temporary == right_temporary
}

pub(super) fn message_key(row: &Value) -> Option<String> {
    let server = server_id(row);
    if !server.is_empty() {
        return Some(format!("server:{server}"));
    }
    let temporary = temporary_id(row);
    if !temporary.is_empty() {
        return Some(format!("temporary:{temporary}"));
    }
    None
}

pub(super) fn server_id(row: &Value) -> &str {
    ["id", "msgId", "postId", "post_id"]
        .into_iter()
        .find_map(|key| {
            row.get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
}

fn temporary_id(row: &Value) -> &str {
    ["temporary_id", "temporaryId"]
        .into_iter()
        .find_map(|key| {
            row.get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
}

fn create_at(row: &Value) -> i64 {
    row.get("create_at")
        .or_else(|| row.get("createAt"))
        .or_else(|| row.get("createdAt"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// 以 storage scan 的 canonical 时间倒序稳定排列 readback rows。
pub(crate) fn sort_recent_rows_desc(rows: &mut [Value]) {
    rows.sort_by(|left, right| {
        create_at(right).cmp(&create_at(left)).then_with(|| {
            match (message_key(right), message_key(left)) {
                (Some(right), Some(left)) => right.cmp(&left),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        })
    });
}

#[cfg(test)]
mod readback_overlay_tests {
    use super::*;
    use helix_core::effect::StorageOp;
    use serde_json::json;

    fn channel() -> ChannelId {
        crate::state::test_channel_id(42)
    }

    fn local_rich_row(channel_id: &str) -> Value {
        json!({
            "id": "post-1",
            "temporary_id": "tmp-1",
            "channel_id": channel_id,
            "user_id": "u-local",
            "type": "TEXT",
            "message": "local message",
            "simple_message": "local message",
            "props": "{\"channel_event_seq\":7}",
            "expedite_map": "{\"sender\":{\"id\":\"u-local\"}}",
            "reply_id": "reply-1",
            "reply_root_id": "root-1",
            "reply_first_level_id": "first-1",
            "replied_message": "{\"message\":\"parent\"}",
            "reply_messages": "{\"reply-1\":{\"message\":\"child\"}}",
            "reply_count": 2,
            "read_bits": "01",
            "snapshot_id": "snapshot-1",
            "mentions": "[\"u-local\"]",
            "topic": "{\"topicId\":\"topic-1\"}",
            "quick_reply": "[{\"emoji\":\"thumb\",\"userIds\":[\"u-local\"]}]"
        })
    }

    fn spec_from(op: StorageOp) -> helix_core::effect::UpsertSpec {
        let StorageOp::BatchUpsert(spec) = op else {
            panic!("readback must emit BatchUpsert");
        };
        spec
    }

    /// Missing remote rich keys preserve both the UI row and the SQL conflict update columns.
    #[test]
    fn readback_missing_rich_fields_overlay_local_and_guard_upsert() {
        let channel = channel();
        let local = local_rich_row(channel.as_str());
        let remote = json!({
            "id": "post-1",
            "temporaryId": "tmp-1",
            "channelId": channel.as_str(),
            "userId": "u-remote",
            "type": "TEXT",
            "message": "remote message",
            "simpleMessage": "remote message",
            "viewers": ["all"]
        });

        let (rows, ops) = visible_remote_rows_and_cache_ops(
            channel,
            vec![remote],
            std::slice::from_ref(&local),
            "",
        )
        .expect("missing-field readback must be renderable");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["expediteMap"]["sender"]["id"], "u-local");
        assert_eq!(rows[0]["replyId"], "reply-1");
        assert_eq!(rows[0]["replyCount"], 2);
        assert_eq!(rows[0]["replyMessages"]["reply-1"]["message"], "child");
        assert_eq!(rows[0]["readBits"], "01");
        assert_eq!(rows[0]["snapshotId"], "snapshot-1");
        assert_eq!(rows[0]["mentions"][0], "u-local");
        assert_eq!(rows[0]["topic"]["topicId"], "topic-1");
        assert_eq!(rows[0]["quickReply"][0]["emoji"], "thumb");
        assert_eq!(rows[0]["props"]["channel_event_seq"], 7);

        let spec = spec_from(ops.into_iter().next().expect("one readback op"));
        for column in [
            "expedite_map",
            "reply_id",
            "reply_root_id",
            "reply_first_level_id",
            "replied_message",
            "reply_messages",
            "reply_count",
            "read_bits",
            "snapshot_id",
            "mentions",
            "topic",
            "quick_reply",
            "props",
        ] {
            assert!(
                spec.exclude_from_update.contains(&column),
                "missing column {column} must be guarded"
            );
        }
    }

    /// Explicit empty rich fields remain authoritative in readback, while storage preserves type-6 read bits.
    #[test]
    fn readback_explicit_empty_rich_fields_render_empty_but_preserve_stored_read_bits() {
        let channel = channel();
        let local = local_rich_row(channel.as_str());
        let remote = json!({
            "id": "post-1",
            "temporaryId": "tmp-1",
            "channelId": channel.as_str(),
            "userId": "u-remote",
            "type": "TEXT",
            "message": "remote message",
            "simpleMessage": "remote message",
            "viewers": ["all"],
            "expediteMap": {},
            "replyId": "",
            "replyRootId": "",
            "replyFirstLevelId": "",
            "repliedMessage": null,
            "replyMessages": {},
            "replyCount": 0,
            "readBits": "",
            "snapshotId": "",
            "mentions": [],
            "topic": {},
            "quickReply": []
        });

        let (rows, ops) = visible_remote_rows_and_cache_ops(
            channel,
            vec![remote],
            std::slice::from_ref(&local),
            "",
        )
        .expect("explicit-empty readback must be renderable");
        assert_eq!(rows[0]["expediteMap"], json!({}));
        assert_eq!(rows[0]["replyId"], "");
        assert_eq!(rows[0]["replyMessages"], json!({}));
        assert_eq!(rows[0]["replyCount"], 0);
        assert_eq!(rows[0]["readBits"], "");
        assert_eq!(rows[0]["snapshotId"], "");
        assert_eq!(rows[0]["mentions"], json!([]));
        assert_eq!(rows[0]["topic"], json!({}));
        assert_eq!(rows[0]["quickReply"], json!([]));

        let spec = spec_from(ops.into_iter().next().expect("one readback op"));
        for column in [
            "expedite_map",
            "reply_id",
            "reply_root_id",
            "reply_first_level_id",
            "replied_message",
            "reply_messages",
            "reply_count",
            "snapshot_id",
            "mentions",
            "topic",
            "quick_reply",
        ] {
            assert!(
                !spec.exclude_from_update.contains(&column),
                "explicit column {column} must remain writable"
            );
        }
        assert!(
            spec.exclude_from_update.contains(&"read_bits"),
            "empty type-1 read_bits must not erase the dedicated type-6 projection"
        );
    }

    /// Complete V3 payload wins over a different local rich projection and needs no guards.
    #[test]
    fn complete_v3_readback_prefers_remote_rich_fields() {
        let channel = channel();
        let local = local_rich_row(channel.as_str());
        let remote = json!({
            "id": "post-1",
            "temporaryId": "tmp-1",
            "channelId": channel.as_str(),
            "userId": "u-remote",
            "type": "TEXT",
            "message": "remote message",
            "simpleMessage": "remote message",
            "viewers": ["all"],
            "expediteMap": {"sender":{"id":"u-remote"}},
            "replyId": "reply-remote",
            "replyRootId": "root-remote",
            "replyFirstLevelId": "first-remote",
            "repliedMessage": {"message":"remote parent"},
            "replyMessages": {"reply-remote":{"message":"remote child"}},
            "replyCount": 1,
            "readBits": "10",
            "snapshotId": "snapshot-remote",
            "mentions": ["u-remote"],
            "topic": {"topicId":"topic-remote"},
            "quickReply": [{"emoji":"wave","userIds":["u-remote"]}]
        });

        let (rows, ops) = visible_remote_rows_and_cache_ops(
            channel,
            vec![remote],
            std::slice::from_ref(&local),
            "",
        )
        .expect("complete V3 readback must be renderable");
        assert_eq!(rows[0]["expediteMap"]["sender"]["id"], "u-remote");
        assert_eq!(rows[0]["replyId"], "reply-remote");
        assert_eq!(rows[0]["replyCount"], 1);
        assert_eq!(
            rows[0]["replyMessages"]["reply-remote"]["message"],
            "remote child"
        );
        assert_eq!(rows[0]["readBits"], "10");

        let spec = spec_from(ops.into_iter().next().expect("one readback op"));
        for column in [
            "expedite_map",
            "reply_id",
            "reply_messages",
            "reply_count",
            "read_bits",
            "snapshot_id",
            "mentions",
            "topic",
            "quick_reply",
        ] {
            assert!(
                !spec.exclude_from_update.contains(&column),
                "complete field {column} must remain writable"
            );
        }
    }
}
