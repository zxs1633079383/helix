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

pub(super) fn parse_local_rows(bytes: &[u8]) -> Result<Vec<Value>, ImError> {
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

pub(super) fn visible_remote_rows_and_cache_ops(
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
        if post_event_seq(&post).is_none() {
            let local_sequence = local_sequences
                .server
                .get(fields.id.as_str())
                .or_else(|| local_sequences.temporary.get(fields.temporary_id.as_str()))
                .copied();
            if let Some(sequence) = local_sequence {
                preserve_local_event_sequence(&mut post, sequence);
                fields = crate::ws::parser::extract_post_fields(&post);
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
        ops.push(crate::channel::event_to_upsert_op(&event));
        rows.push(post);
    }
    Ok((rows, ops))
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
    if !fields.props.is_empty() {
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

pub(super) fn sort_recent_rows_desc(rows: &mut [Value]) {
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
