use crate::module::ImModule;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, CorrelationContext, SendStatus, TemporaryId};
use crate::ImError;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod rehydrate;
use rehydrate::{decode_single_row, rebuild_body, row_str};

#[derive(Debug, Deserialize, Serialize)]
struct RetrySendCommand {
    #[serde(alias = "messageId")]
    message_id: String,
    #[serde(default, alias = "reqId")]
    req_id: Option<String>,
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn media_retry_without_java_base_fails_before_mutating_failed_state() {
        let mut module = ImModule::new(crate::module::ImConfig {
            api_base_url: "http://go.test/api/cses".to_string(),
            default_api_base_url: String::new(),
            ..Default::default()
        });
        let temporary_id = TemporaryId("temporary-media-retry".to_string());
        let channel_id =
            ChannelId::from_str("ch00000000000000000000000a").expect("valid test channel");
        let mut pending = PendingSend::new(
            temporary_id.clone(),
            helix_core::TimerId::from_raw(91),
            None,
        );
        pending.status = SendStatus::UnSend;
        pending.upload_failed = true;
        pending.remaining_uploads = 1;
        pending.body = Some(serde_json::json!({
            "temporaryId": temporary_id.0.as_str(),
            "channelId": channel_id.as_str(),
            "type": "FILE",
            "message": "",
            "props": {
                "file": {
                    "name": "report.pdf",
                    "upload": {"status": "failed"}
                }
            }
        }));
        module
            .state
            .pending_sends
            .insert(temporary_id.clone(), pending);
        module.state.failed_media_ops.insert(
            (
                temporary_id.clone(),
                crate::send::upload_props::UploadTarget::File,
            ),
            crate::send::upload_props::FailedMediaOp::Prepare(
                crate::send::upload_props::PendingMediaPrepare {
                    temporary_id: temporary_id.clone(),
                    channel_id,
                    target: crate::send::upload_props::UploadTarget::File,
                    input: crate::send::upload_props::MediaInput {
                        local_path: "/tmp/report.pdf".to_string(),
                        file_name: "report.pdf".to_string(),
                        content_type: "application/pdf".to_string(),
                        size: 9,
                        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    },
                },
            ),
        );

        let mut out = EffectSink::new();
        let error = handle_retry_send(
            &mut module,
            br#"{"message_id":"temporary-media-retry","req_id":"retry-request"}"#,
            1_000,
            &mut out,
        )
        .expect_err("missing Java base must fail closed");

        assert!(error.to_string().contains("Java API base is required"));
        assert!(out.is_empty());
        assert_eq!(
            module
                .state
                .pending_sends
                .get(&temporary_id)
                .map(|pending| pending.status),
            Some(SendStatus::UnSend)
        );
        assert_eq!(module.state.failed_media_ops.len(), 1);
        assert!(!module.state.media_retry_inflight.contains(&temporary_id));
        assert_eq!(
            module
                .state
                .pending_sends
                .get(&temporary_id)
                .and_then(|pending| pending.timeline_readback.causation_id.as_deref()),
            None
        );
    }
}

pub(crate) fn handle_retry_send(
    module: &mut ImModule,
    payload: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    handle_retry_send_with_window(module, payload, now_ms, None, None, out)
}

/// 解析 retry 请求并按内存或本地存储事实恢复发送。
fn handle_retry_send_with_window(
    module: &mut ImModule,
    payload: &[u8],
    now_ms: u64,
    action_channel_id: Option<ChannelId>,
    window_token: Option<String>,
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
        let result = retry_from_memory(
            module,
            temporary_id,
            cmd.req_id.clone(),
            action_channel_id,
            window_token.clone(),
            out,
        );
        return finish_retry_action_error(
            module,
            result,
            action_channel_id,
            cmd.req_id,
            window_token,
            out,
        );
    }

    emit_durable_lookup(
        module,
        cmd.message_id,
        now_ms,
        cmd.req_id,
        action_channel_id,
        window_token,
        false,
        out,
    );
    Ok(())
}

/// 从 O(1) pending 索引恢复失败发送，并把重复点击收敛为同一进行中 attempt。
fn retry_from_memory(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    request_id: Option<String>,
    action_channel_id: Option<ChannelId>,
    window_token: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if module.state.media_retry_inflight.contains(&temporary_id) {
        return schedule_retry_action_terminal_readback(
            module,
            action_channel_id,
            request_id,
            window_token,
            out,
        );
    }
    let media_failed = module
        .state
        .pending_sends
        .get(&temporary_id)
        .is_some_and(|pending| pending.upload_failed);
    if media_failed {
        return retry_failed_media(module, temporary_id, request_id, window_token, out);
    }
    let retry_inflight = module
        .state
        .pending_sends
        .get(&temporary_id)
        .is_some_and(|pending| matches!(pending.status, SendStatus::Local | SendStatus::Sending));
    if retry_inflight {
        return schedule_retry_action_terminal_readback(
            module,
            action_channel_id,
            request_id,
            window_token,
            out,
        );
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
                Some("file" | "FILE" | "rich" | "IMAGE")
            )
        });
        if media_body && pending.remaining_uploads == 0 && pending.status != SendStatus::UnSend {
            return schedule_retry_action_terminal_readback(
                module,
                action_channel_id,
                request_id,
                window_token,
                out,
            );
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
    continue_retry(module, temporary_id, body, request_id, window_token, out)
}

fn retry_failed_media(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    request_id: Option<String>,
    window_token: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    use crate::send::upload_props::{FailedMediaOp, UploadTarget};

    // 旧 FFI ABI 或缺省 host 配置可能恢复出 durable media journal，但没有 Java base。
    // 必须在 remove failed op / 修改 PendingSend / 登记 inflight 之前失败关闭，否则一次
    // 无效 retry 会把内存状态卡成永久 Sending，直到进程重启。
    crate::send::upload_props::validate_java_api_base_url(&module.config.default_api_base_url)?;

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
        .iter()
        .filter_map(|key| module.state.failed_media_ops.get(key).cloned())
        .collect::<Vec<_>>();
    if failed.is_empty() {
        if let Some(channel_id) = module
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.body.as_ref())
            .and_then(|body| body.get("channelId"))
            .and_then(Value::as_str)
            .and_then(ChannelId::from_str)
        {
            module.refresh_attached_timeline(
                channel_id,
                window_token.as_deref().unwrap_or("latest"),
                request_id,
                out,
            )?;
        }
        return Ok(());
    }

    let (body_after, props_json, channel_id, timeline_readback) = {
        let pending = module
            .state
            .pending_sends
            .get(&temporary_id)
            .ok_or_else(|| {
                ImError::Parse("im_retry_send missing pending media send".to_string())
            })?;
        if pending.status != SendStatus::UnSend {
            return Err(ImError::Parse(format!(
                "im_retry_send requires failed media status, got {:?}",
                pending.status
            )));
        }
        let mut body_after = pending.body.clone().ok_or_else(|| {
            ImError::Parse("im_retry_send missing pending media body".to_string())
        })?;
        let channel_id = body_after
            .get("channelId")
            .and_then(Value::as_str)
            .and_then(ChannelId::from_str)
            .ok_or_else(|| {
                ImError::Parse("im_retry_send pending media body missing channelId".to_string())
            })?;
        let props = body_after
            .get_mut("props")
            .ok_or_else(|| ImError::Parse("im_retry_send missing media props".to_string()))?;
        for operation in &failed {
            let status = match operation {
                FailedMediaOp::Prepare(_) => "preparing",
                FailedMediaOp::Complete(_) => "completing",
            };
            crate::send::upload_props::mark_media_stage(props, operation.target(), status)?;
        }
        // 用户显式 retry 的单次边界整体复制：后续 progress / terminal event 必须同时保留
        // 授权 window 与 causation，不能让二者分叉。
        let timeline_readback = crate::pending_send::TimelineReadbackContext {
            window_token,
            causation_id: request_id.or_else(|| pending.timeline_readback.causation_id.clone()),
        };
        let props_json =
            serde_json::to_string(props).map_err(|error| ImError::Serialize(error.to_string()))?;
        (body_after, props_json, channel_id, timeline_readback)
    };

    let persist_corr = module.alloc_corr_internal();
    let reset_file_progress = failed
        .iter()
        .any(|operation| matches!(operation.target(), UploadTarget::File));
    out.push(Effect::PersistAtomic {
        corr: persist_corr,
        ops: vec![crate::send::upload_props::media_send_state_persist_op(
            &temporary_id,
            props_json,
            "sending",
            reset_file_progress.then_some(0),
        )],
    });
    module.state.pending_media_retry_resets.insert(
        persist_corr,
        crate::send::upload_props::PendingMediaRetryReset {
            temporary_id: temporary_id.clone(),
            channel_id,
            body_after,
            timeline_readback,
            operations: failed,
        },
    );
    module
        .state
        .media_retry_inflight
        .insert(temporary_id.clone());

    Ok(())
}

pub(crate) fn handle_media_retry_reset_reply(
    module: &mut ImModule,
    reset: crate::send::upload_props::PendingMediaRetryReset,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    use crate::send::upload_props::FailedMediaOp;

    match outcome {
        PortOutcome::Ok(_) => {
            let pending = module
                .state
                .pending_sends
                .get_mut(&reset.temporary_id)
                .ok_or_else(|| {
                    ImError::Parse(
                        "media retry reset acknowledged without pending send".to_string(),
                    )
                })?;
            pending.body = Some(reset.body_after);
            pending.status = SendStatus::Sending;
            pending.upload_failed = false;
            pending.timeline_readback = reset.timeline_readback.clone(); // hot-path-audit: cold (explicit retry boundary)

            for operation in &reset.operations {
                module
                    .state
                    .failed_media_ops
                    .remove(&(reset.temporary_id.clone(), operation.target().clone()));
            }

            if let Err(error) = module.refresh_attached_timeline(
                reset.channel_id,
                reset
                    .timeline_readback
                    .window_token
                    .as_deref()
                    .unwrap_or("latest"),
                reset.timeline_readback.causation_id,
                out,
            ) {
                tracing::warn!(
                    tmp_id = reset.temporary_id.0.as_str(),
                    error = ?error,
                    "media retry reset committed but timeline refresh could not be scheduled"
                );
            }

            for operation in reset.operations {
                match operation {
                    FailedMediaOp::Prepare(pending) => module.emit_media_prepare(
                        pending.channel_id,
                        pending.temporary_id,
                        pending.target,
                        pending.input,
                        out,
                    )?,
                    FailedMediaOp::Complete(pending) => {
                        module.finalize_or_queue_media_completion(pending, out)?;
                    }
                }
            }
        }
        PortOutcome::Err(error) => {
            module
                .state
                .media_retry_inflight
                .remove(&reset.temporary_id);
            tracing::warn!(
                tmp_id = reset.temporary_id.0.as_str(),
                error = ?error,
                "media retry reset persist failed; Java/OSS I/O suppressed"
            );
            if let Err(refresh_error) = module.refresh_attached_timeline(
                reset.channel_id,
                reset
                    .timeline_readback
                    .window_token
                    .as_deref()
                    .unwrap_or("latest"),
                reset.timeline_readback.causation_id,
                out,
            ) {
                tracing::warn!(
                    tmp_id = reset.temporary_id.0.as_str(),
                    error = ?refresh_error,
                    "media retry reset failure timeline refresh could not be scheduled"
                );
            }
        }
    }
    Ok(())
}

fn schedule_retry_action_terminal_readback(
    module: &mut ImModule,
    action_channel_id: Option<ChannelId>,
    request_id: Option<String>,
    window_token: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let (Some(channel_id), Some(request_id)) = (action_channel_id, request_id) else {
        return Ok(());
    };
    module.refresh_attached_timeline(
        channel_id,
        window_token.as_deref().unwrap_or("latest"),
        Some(request_id),
        out,
    )
}

fn finish_retry_action_error(
    module: &mut ImModule,
    result: Result<(), ImError>,
    action_channel_id: Option<ChannelId>,
    request_id: Option<String>,
    window_token: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if action_channel_id.is_some() && request_id.is_some() => {
            tracing::warn!(error = ?error, "accepted retry action failed; scheduling causal terminal readback");
            schedule_retry_action_terminal_readback(
                module,
                action_channel_id,
                request_id,
                window_token,
                out,
            )
        }
        Err(error) => Err(error),
    }
}

fn emit_durable_lookup(
    module: &mut ImModule,
    message_id: String,
    requested_at_ms: u64,
    request_id: Option<String>,
    action_channel_id: Option<ChannelId>,
    window_token: Option<String>,
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
            action_channel_id,
            window_token,
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
    action_channel_id: Option<ChannelId>,
    window_token: Option<String>,
    lookup_by_id: bool,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let terminal_request_id = request_id.clone();
    let terminal_window_token = window_token.clone();
    let bytes = match outcome {
        PortOutcome::Ok(reply) => reply.0.as_ref(),
        PortOutcome::Err(error) => {
            tracing::warn!(
                message_id,
                lookup_by_id,
                error = ?error,
                "durable retry message lookup failed"
            );
            return schedule_retry_action_terminal_readback(
                module,
                action_channel_id,
                terminal_request_id,
                terminal_window_token,
                out,
            );
        }
    };
    let row = match decode_single_row(bytes) {
        Ok(row) => row,
        Err(error) => {
            return finish_retry_action_error(
                module,
                Err(error),
                action_channel_id,
                terminal_request_id,
                terminal_window_token,
                out,
            );
        }
    };
    let Some(row) = row else {
        if !lookup_by_id {
            emit_durable_lookup(
                module,
                message_id,
                requested_at_ms,
                request_id,
                action_channel_id,
                window_token,
                true,
                out,
            );
            return Ok(());
        }
        return finish_retry_action_error(
            module,
            Err(ImError::Parse(
                "im_retry_send durable message not found".to_string(),
            )),
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    };

    let temporary_id = row_str(&row, "temporary_id");
    if temporary_id.is_empty() {
        return finish_retry_action_error(
            module,
            Err(ImError::Parse(
                "im_retry_send durable message missing temporary_id".to_string(),
            )),
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    }
    let temporary_id = TemporaryId(temporary_id.to_string());
    let body = match rebuild_body(module, &row, &temporary_id, requested_at_ms) {
        Ok(body) => body,
        Err(error) => {
            return finish_retry_action_error(
                module,
                Err(error),
                action_channel_id,
                terminal_request_id,
                terminal_window_token,
                out,
            );
        }
    };

    let durable_media_count = module
        .state
        .failed_media_ops
        .keys()
        .filter(|(message_id, _)| message_id == &temporary_id)
        .count();
    if durable_media_count > 0 {
        let result = continue_durable_media_retry(
            module,
            temporary_id,
            body,
            request_id,
            window_token,
            durable_media_count,
            out,
        );
        return finish_retry_action_error(
            module,
            result,
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    }

    let status = row_str(&row, "send_status");
    if status == "sent" {
        return schedule_retry_action_terminal_readback(
            module,
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    }
    if !matches!(status, "unsend" | "failed") {
        return finish_retry_action_error(
            module,
            Err(ImError::Parse(format!(
                "im_retry_send durable message requires failed status, got {status}"
            ))),
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    }
    if is_media_body(&body) && !media_body_is_fully_completed(&body) {
        let reason = if module.state.media_recovery_ready {
            "im_retry_send incomplete media is missing durable pending_media journal"
        } else {
            "im_retry_send media recovery scan is not ready"
        };
        return finish_retry_action_error(
            module,
            Err(ImError::Parse(reason.to_string())),
            action_channel_id,
            terminal_request_id,
            terminal_window_token,
            out,
        );
    }

    let result = continue_retry(module, temporary_id, body, request_id, window_token, out);
    finish_retry_action_error(
        module,
        result,
        action_channel_id,
        terminal_request_id,
        terminal_window_token,
        out,
    )
}

fn is_media_body(body: &Value) -> bool {
    matches!(
        body.get("type").and_then(Value::as_str),
        Some("file" | "FILE" | "rich" | "IMAGE")
    )
}

// 判断 durable body 是否已经只含稳定 bucket/uri，可直接重放 posts/create。
fn media_body_is_fully_completed(body: &Value) -> bool {
    let Some(props) = body.get("props") else {
        return false;
    };
    match body.get("type").and_then(Value::as_str) {
        Some("file" | "FILE") => props
            .get("file")
            .is_some_and(|node| completed_media_node(node, "attachment")),
        Some("rich" | "IMAGE") => {
            props
                .get("files")
                .and_then(Value::as_array)
                .is_some_and(|files| {
                    !files.is_empty()
                        && files
                            .iter()
                            .all(|node| completed_media_node(node, "picture"))
                })
        }
        _ => true,
    }
}

// 校验单个完成媒体节点不再含临时 URL，并与消息类型要求的 bucket 一致。
fn completed_media_node(node: &Value, expected_bucket: &str) -> bool {
    let non_empty = |key: &str| {
        node.get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    };
    node.pointer("/upload/status").and_then(Value::as_str) == Some("completed")
        && node.get("bucket").and_then(Value::as_str) == Some(expected_bucket)
        && non_empty("id")
        && non_empty("name")
        && non_empty("contentType")
        && node
            .get("size")
            .and_then(Value::as_u64)
            .is_some_and(|size| size > 0)
        && node
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(|sha| {
                sha.len() == 64
                    && sha
                        .as_bytes()
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
        && non_empty("uri")
        && node.get("url").is_none()
}

fn continue_durable_media_retry(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    body: Value,
    request_id: Option<String>,
    window_token: Option<String>,
    remaining_uploads: usize,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    crate::send::upload_props::validate_java_api_base_url(&module.config.default_api_base_url)?;
    let timeout_timer = module.alloc_timer();
    let mut pending = PendingSend::new(
        temporary_id.clone(),
        timeout_timer,
        module.state.connection_id.clone(),
    );
    pending.status = SendStatus::UnSend;
    pending.timeline_readback.causation_id = request_id.clone();
    pending.body = Some(body);
    pending.remaining_uploads = remaining_uploads;
    pending.upload_failed = true;
    module
        .state
        .pending_sends
        .insert(temporary_id.clone(), pending);
    retry_failed_media(module, temporary_id, request_id, window_token, out)
}

// Re-enters the ordinary P1 barrier while preserving the failed message identity and body.
fn continue_retry(
    module: &mut ImModule,
    temporary_id: TemporaryId,
    body: Value,
    request_id: Option<String>,
    window_token: Option<String>,
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
    let persist_corr = module.alloc_corr_internal();
    let connection_id = module.state.connection_id.clone();
    let pending = module
        .state
        .pending_sends
        .entry(temporary_id.clone())
        .or_insert_with(|| PendingSend::new(temporary_id.clone(), timeout_timer, connection_id));
    pending.status = SendStatus::Local;
    pending.timeout_timer = timeout_timer;
    pending.http_started = false;
    pending.authoritative_readback_after_http = true;
    pending.body = Some(body.clone());
    if request_id.is_some() {
        pending.timeline_readback.causation_id = request_id;
    }
    pending.timeline_readback.window_token = window_token;

    pending.persist_corr = Some(persist_corr);
    module.state.corr_map.insert(
        persist_corr,
        CorrelationContext::OptimisticSend {
            temporary_id: temporary_id.clone(),
        },
    );
    out.push(Effect::Persist {
        corr: persist_corr,
        ops: vec![crate::pending_send::send_status_persist_op(
            &temporary_id,
            "sending",
        )],
    });
    out.push(
        crate::event::post::sending_from_local_body(
            channel_id.as_str(),
            &temporary_id.0,
            module.config.auth_user_id.as_str(),
            &body,
        )?
        .into_effect(),
    );
    Ok(())
}
