use crate::module::ImModule;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, CorrelationContext, SendStatus, TemporaryId};
use crate::ImError;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct RetrySendCommand {
    #[serde(alias = "messageId")]
    message_id: String,
    #[serde(default, alias = "reqId")]
    req_id: Option<String>,
}

pub(crate) fn handle_retry_send(
    module: &mut ImModule,
    payload: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let cmd: RetrySendCommand = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_retry_send payload: {e}")))?;
    if cmd.message_id.is_empty() {
        return Err(ImError::Parse(
            "im_retry_send missing message_id".to_string(),
        ));
    }
    let temporary_id = TemporaryId(cmd.message_id.clone());
    if module.state.pending_sends.contains_key(&temporary_id) {
        return retry_from_memory(module, temporary_id, cmd.req_id, out);
    }

    emit_durable_lookup(module, cmd.message_id, now_ms, cmd.req_id, false, out);
    Ok(())
}

fn retry_from_memory(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    request_id: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let body = {
        let pending = module
            .state
            .pending_sends
            .get(&temporary_id)
            .ok_or_else(|| ImError::Parse("im_retry_send missing pending send".to_string()))?;
        if pending.upload_failed {
            return Err(ImError::Parse(
                "im_retry_send cannot replace im_retry_upload".to_string(),
            ));
        }
        if pending.status != SendStatus::UnSend {
            return Err(ImError::Parse(format!(
                "im_retry_send requires failed status, got {:?}",
                pending.status
            )));
        }
        pending
            .body
            .clone()
            .ok_or_else(|| ImError::Parse("im_retry_send missing cached send body".to_string()))?
    };
    continue_retry(module, temporary_id, body, request_id, out)
}

fn emit_durable_lookup(
    module: &mut ImModule,
    message_id: String,
    requested_at_ms: u64,
    request_id: Option<String>,
    lookup_by_id: bool,
    out: &mut EffectSink,
) {
    let corr = module.alloc_corr_internal();
    let key_col = if lookup_by_id { "id" } else { "temporary_id" };
    module.state.corr_map.insert(
        corr,
        CorrelationContext::RetrySendRehydrate {
            message_id: message_id.clone(),
            requested_at_ms,
            request_id,
            lookup_by_id,
        },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![StorageOp::Get(GetSpec {
            table: "message",
            key_col,
            key_val: SqlValue::Text(message_id),
        })],
    });
}

pub(crate) fn handle_rehydrate_reply(
    module: &mut ImModule,
    message_id: String,
    requested_at_ms: u64,
    request_id: Option<String>,
    lookup_by_id: bool,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let bytes = match outcome {
        PortOutcome::Ok(reply) => reply.0.as_ref(),
        PortOutcome::Err(error) => {
            tracing::warn!(
                message_id,
                lookup_by_id,
                error = ?error,
                "durable retry message lookup failed"
            );
            return Ok(());
        }
    };
    let row = decode_single_row(bytes)?;
    let Some(row) = row else {
        if !lookup_by_id {
            emit_durable_lookup(module, message_id, requested_at_ms, request_id, true, out);
            return Ok(());
        }
        return Err(ImError::Parse(
            "im_retry_send durable message not found".to_string(),
        ));
    };

    let status = row_str(&row, "send_status");
    if !matches!(status, "unsend" | "failed") {
        return Err(ImError::Parse(format!(
            "im_retry_send durable message requires failed status, got {status}"
        )));
    }
    let temporary_id = row_str(&row, "temporary_id");
    if temporary_id.is_empty() {
        return Err(ImError::Parse(
            "im_retry_send durable message missing temporary_id".to_string(),
        ));
    }
    let temporary_id = TemporaryId(temporary_id.to_string());
    let body = rebuild_body(module, &row, &temporary_id, requested_at_ms)?;

    continue_retry(module, temporary_id, body, request_id, out)
}

fn continue_retry(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    body: Value,
    request_id: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let channel_id = body
        .get("channelId")
        .and_then(Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("im_retry_send cached body missing channelId".to_string()))?;
    let body_temporary_id = body
        .get("temporaryId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ImError::Parse("im_retry_send cached body missing temporaryId".to_string())
        })?;
    if body_temporary_id != temporary_id.0 {
        return Err(ImError::Parse(
            "im_retry_send cached body correlation mismatch".to_string(),
        ));
    }

    let timeout_timer = module.alloc_timer();
    let connection_id = module.state.connection_id.clone();
    let pending = module
        .state
        .pending_sends
        .entry(temporary_id.clone())
        .or_insert_with(|| PendingSend::new(temporary_id.clone(), timeout_timer, connection_id));
    pending.status = SendStatus::Sending;
    pending.timeout_timer = timeout_timer;
    pending.body = Some(body.clone());
    if request_id.is_some() {
        pending.request_id = request_id;
    }

    out.push(Effect::PersistFire {
        ops: vec![crate::pending_send::send_status_persist_op(
            &temporary_id,
            "sending",
        )],
    });
    out.push(crate::acl::to_effect::emit_post_sending(
        channel_id,
        &temporary_id.0,
        body.get("message").and_then(Value::as_str).unwrap_or(""),
        body.get("type").and_then(Value::as_str).unwrap_or("TEXT"),
    ));
    module.emit_posts_create_http(channel_id, temporary_id, &body, out)
}

fn decode_single_row(bytes: &[u8]) -> Result<Option<Value>, ImError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let rows: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ImError::Parse(format!("im_retry_send durable row: {error}")))?;
    Ok(rows.into_iter().next().filter(Value::is_object))
}

fn rebuild_body(
    module: &ImModule,
    row: &Value,
    temporary_id: &TemporaryId,
    requested_at_ms: u64,
) -> Result<Value, ImError> {
    use crate::outbound::send_build::{build_message_object, BuildInput, UserIdentity};

    let channel_id = row_str(row, "channel_id");
    ChannelId::from_str(channel_id).ok_or_else(|| {
        ImError::Parse("im_retry_send durable message missing channel_id".to_string())
    })?;
    let msg_type = match row_str(row, "type") {
        "" => "TEXT",
        value => value,
    };
    let props = row_json(row, "props", json!({}));
    let viewers = row_json(row, "viewers", json!([]))
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mentions = row_json(row, "mentions", json!([]));
    let create_at = row
        .get("create_at")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(requested_at_ms);
    let identity = module.config.user_identity();
    let mut body = build_message_object(&BuildInput {
        channel_id,
        temporary_id: temporary_id.0.as_str(),
        msg_type,
        post_id: row_str(row, "id"),
        text: row_str(row, "message"),
        viewers,
        mentions,
        props,
        topic_id: row_str(row, "topic"),
        replied: None,
        now_ms: create_at,
        identity: UserIdentity {
            user_id: identity.user_id,
            team_id: identity.team_id,
            user_name: identity.user_name,
            org_name: identity.org_name,
            dept_name: identity.dept_name,
        },
    })
    .ok_or_else(|| ImError::Parse("im_retry_send durable message is empty".to_string()))?;

    for (column, wire_key) in [
        ("reply_id", "replyId"),
        ("reply_root_id", "replyRootId"),
        ("reply_first_level_id", "replyFirstLevelId"),
    ] {
        let value = row_str(row, column);
        if !value.is_empty() {
            body[wire_key] = Value::String(value.to_string());
        }
    }
    let snapshot = row_json(row, "user_snapshot", json!({}));
    if snapshot.as_object().is_some_and(|value| !value.is_empty()) {
        if let Some(team_id) = snapshot.get("teamId").and_then(Value::as_str) {
            body["teamId"] = Value::String(team_id.to_string());
        }
        body["userSnapshot"] = snapshot;
    }
    let user_id = row_str(row, "user_id");
    if !user_id.is_empty() {
        body["userId"] = Value::String(user_id.to_string());
    }
    Ok(body)
}

fn row_str<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_json(row: &Value, key: &str, fallback: Value) -> Value {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(fallback)
}
