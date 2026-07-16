use crate::module::ImModule;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, CorrelationContext, SendStatus, TemporaryId};
use crate::ImError;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};
use serde::Deserialize;
use serde_json::Value;

mod rehydrate;
use rehydrate::{decode_single_row, rebuild_body, row_str};

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
    if module.state.media_retry_inflight.contains(&temporary_id) {
        return Ok(());
    }
    let media_failed = module
        .state
        .pending_sends
        .get(&temporary_id)
        .is_some_and(|pending| pending.upload_failed);
    if media_failed {
        return retry_failed_media(module, temporary_id, out);
    }
    let body = {
        let pending = module
            .state
            .pending_sends
            .get(&temporary_id)
            .ok_or_else(|| ImError::Parse("im_retry_send missing pending send".to_string()))?;
        let media_body = pending.body.as_ref().is_some_and(|body| {
            matches!(
                body.get("type").and_then(Value::as_str),
                Some("file" | "FILE" | "rich")
            )
        });
        if media_body && pending.remaining_uploads == 0 && pending.status != SendStatus::UnSend {
            return Ok(());
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

fn retry_failed_media(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    use crate::send::upload_props::{FailedMediaOp, PendingMediaOp, UploadTarget};

    let mut keys = module
        .state
        .failed_media_ops
        .keys()
        .filter(|(message_id, _)| message_id == &temporary_id)
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by_key(|(_, target)| match target {
        UploadTarget::RichImage { index } => (0u8, *index),
        UploadTarget::File => (1u8, 0usize),
    });
    let failed = keys
        .into_iter()
        .filter_map(|key| module.state.failed_media_ops.remove(&key))
        .collect::<Vec<_>>();
    if failed.is_empty() {
        return Ok(());
    }

    let props_json = {
        let pending = module
            .state
            .pending_sends
            .get_mut(&temporary_id)
            .ok_or_else(|| {
                ImError::Parse("im_retry_send missing pending media send".to_string())
            })?;
        let body = pending.body.as_mut().ok_or_else(|| {
            ImError::Parse("im_retry_send missing pending media body".to_string())
        })?;
        let props = body
            .get_mut("props")
            .ok_or_else(|| ImError::Parse("im_retry_send missing media props".to_string()))?;
        for operation in &failed {
            let status = match operation {
                FailedMediaOp::Prepare(_) => "preparing",
                FailedMediaOp::Put(_) => "uploading",
                FailedMediaOp::Complete(_) => "completing",
            };
            crate::send::upload_props::mark_media_stage(props, operation.target(), status)?;
        }
        pending.upload_failed = false;
        serde_json::to_string(props).map_err(|error| ImError::Serialize(error.to_string()))?
    };

    let persist_corr = module.alloc_corr_internal();
    out.push(Effect::Persist {
        corr: persist_corr,
        ops: vec![crate::pending_send::props_persist_op(
            &temporary_id,
            props_json,
        )],
    });
    module
        .state
        .media_retry_inflight
        .insert(temporary_id.clone());

    for operation in failed {
        match operation {
            FailedMediaOp::Prepare(pending) => {
                module.emit_media_prepare(
                    pending.channel_id,
                    pending.temporary_id,
                    pending.target,
                    pending.input,
                    out,
                )?;
            }
            FailedMediaOp::Put(pending) => {
                let req = crate::send::upload_props::media_upload_request(
                    &pending.input,
                    &pending.prepared,
                )?;
                let corr = module.alloc_corr_internal();
                module
                    .state
                    .pending_media_ops
                    .insert(corr, PendingMediaOp::Put(pending));
                out.push(Effect::UploadFile { corr, req });
            }
            FailedMediaOp::Complete(pending) => {
                let corr = module.alloc_corr_internal();
                let req = crate::send::upload_props::complete_upload_request(
                    &module.config.api_base_url,
                    &pending.prepared.upload_id,
                    crate::acl::sync_http_effects::session_auth_headers(
                        module.state.connection_id.as_deref(),
                    ),
                )?;
                module
                    .state
                    .waiting_media_complete
                    .insert(pending.prepared.upload_id.clone(), pending.clone());
                module
                    .state
                    .pending_media_ops
                    .insert(corr, PendingMediaOp::Complete(pending));
                out.push(Effect::Http { corr, req });
            }
        }
    }
    Ok(())
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
    out.push(crate::acl::to_effect::emit_post_sending_with_reply(
        channel_id,
        &temporary_id.0,
        body.get("message").and_then(Value::as_str).unwrap_or(""),
        body.get("type").and_then(Value::as_str).unwrap_or("TEXT"),
        body.get("replyId").and_then(Value::as_str).unwrap_or(""),
        body.get("replyRootId")
            .and_then(Value::as_str)
            .unwrap_or(""),
        body.get("replyFirstLevelId")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ));
    module.emit_posts_create_http(channel_id, temporary_id, &body, out)
}
