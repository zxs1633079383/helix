use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::send::upload_props::{
    MultipartBatch, MultipartDrain, MultipartSession, PendingMediaOp, PendingMultipartPut,
    PendingMultipartSign, MULTIPART_MAX_INFLIGHT,
};
use crate::state::{ChannelId, CorrelationContext};
use helix_core::EffectSink;

mod dispatch;
mod dispatch_channel_create;
mod dispatch_core;
mod dispatch_projection;
mod dispatch_read;
mod media_write;
pub(crate) mod message_v3_chain;
pub(crate) mod message_v3_post;
pub(crate) mod message_v3_reaction;
mod message_v3_read;
mod message_v3_revoke;
mod message_v3_schedule;
pub(crate) mod message_v3_template;
pub(crate) mod message_v3_urgent;
mod progress;
mod reaction_unread;

fn multipart_session_from_operation(
    operation: &crate::send::upload_props::PendingMediaOp,
) -> crate::send::upload_props::MultipartSession {
    use crate::send::upload_props::{MultipartSession, PendingMediaOp};
    match operation {
        PendingMediaOp::MultipartParts(session) | PendingMediaOp::MultipartComplete(session) => {
            session.clone()
        }
        PendingMediaOp::MultipartSign(sign) => sign.session.clone(),
        PendingMediaOp::MultipartPut(put) => put.session.clone(),
        PendingMediaOp::MultipartInit(prepare) => MultipartSession {
            temporary_id: prepare.temporary_id.clone(),
            channel_id: prepare.channel_id,
            target: prepare.target.clone(),
            input: prepare.input.clone(),
            client_upload_id: crate::send::upload_props::client_upload_id(
                &prepare.temporary_id.0,
                &prepare.target,
            ),
            upload_id: String::new(),
            file_id: String::new(),
            bucket: prepare.target.bucket().to_string(),
            object_key: String::new(),
            uri: String::new(),
            part_size: crate::send::upload_props::MULTIPART_PART_SIZE_BYTES,
            status: "preparing".to_string(),
            parts: Vec::new(),
        },
        _ => unreachable!("multipart_session_from_operation called for legacy media op"),
    }
}

fn multipart_prepare_from_session(
    session: &crate::send::upload_props::MultipartSession,
) -> crate::send::upload_props::PendingMediaPrepare {
    crate::send::upload_props::PendingMediaPrepare {
        temporary_id: session.temporary_id.clone(),
        channel_id: session.channel_id,
        target: session.target.clone(),
        input: session.input.clone(),
    }
}

fn multipart_operation_id(operation: &PendingMediaOp) -> String {
    match operation {
        PendingMediaOp::MultipartInit(prepare) => {
            crate::send::upload_props::client_upload_id(&prepare.temporary_id.0, &prepare.target)
        }
        PendingMediaOp::MultipartParts(session) | PendingMediaOp::MultipartComplete(session) => {
            session.client_upload_id.clone()
        }
        PendingMediaOp::MultipartSign(sign) => sign.session.client_upload_id.clone(),
        PendingMediaOp::MultipartPut(put) => put.session.client_upload_id.clone(),
        _ => String::new(),
    }
}

impl ImModule {
    fn has_live_multipart_uploads(&self, client_upload_id: &str) -> bool {
        self.state.pending_media_ops.values().any(|operation| {
            matches!(
                operation,
                PendingMediaOp::MultipartPut(put)
                    if put.session.client_upload_id == client_upload_id
            )
        })
    }

    fn has_live_multipart_stage_persists(&self, client_upload_id: &str) -> bool {
        self.state
            .pending_media_stage_persists
            .values()
            .any(|operation| multipart_operation_id(operation) == client_upload_id)
    }

    fn remove_multipart_stage_persists(&mut self, client_upload_id: &str) {
        let stage_corrs = self
            .state
            .pending_media_stage_persists
            .iter()
            .filter_map(|(corr, operation)| {
                (multipart_operation_id(operation) == client_upload_id).then_some(*corr)
            })
            .collect::<Vec<_>>();
        for corr in stage_corrs {
            self.state.pending_media_stage_persists.remove(&corr);
        }
    }

    /// Stop issuing new signed work while retaining sibling UploadFile routes
    /// and already issued checkpoint PersistAtomic routes. Retry or terminal
    /// failure is released only after both classes have produced a reply.
    fn begin_multipart_drain(
        &mut self,
        client_upload_id: String,
        drain: MultipartDrain,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.multipart_batches.remove(&client_upload_id);
        self.state
            .multipart_draining
            .entry(client_upload_id.clone())
            .or_insert(drain);
        self.finish_multipart_drain_if_ready(&client_upload_id, out)
    }

    fn finish_multipart_drain_if_ready(
        &mut self,
        client_upload_id: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self.has_live_multipart_uploads(client_upload_id)
            || self.has_live_multipart_stage_persists(client_upload_id)
        {
            return Ok(());
        }
        let Some(drain) = self.state.multipart_draining.remove(client_upload_id) else {
            return Ok(());
        };
        if let Some(after_ms) = drain.retry_after_ms {
            let timer = self.alloc_timer();
            self.state
                .multipart_retry_timers
                .insert(timer, drain.pending);
            out.push(helix_core::Effect::ScheduleTimer {
                id: timer,
                after_ms,
            });
        } else {
            self.fail_media_operation(
                crate::send::upload_props::FailedMediaOp::Multipart(drain.pending),
                drain.hop,
                &drain.error,
                out,
            )?;
        }
        Ok(())
    }

    /// Drop all in-memory signed work for a session during lifecycle shutdown.
    /// Unlike retry/failure, stop is allowed to revoke all routes because the
    /// host is closing the corresponding transports at the same boundary.
    pub(crate) fn clear_multipart_runtime(&mut self, client_upload_id: &str) {
        self.state.multipart_batches.remove(client_upload_id);
        self.state.multipart_draining.remove(client_upload_id);
        let upload_corrs = self
            .state
            .pending_media_ops
            .iter()
            .filter_map(|(corr, operation)| {
                (multipart_operation_id(operation) == client_upload_id).then_some(*corr)
            })
            .collect::<Vec<_>>();
        for corr in upload_corrs {
            self.state.pending_media_ops.remove(&corr);
            self.finish_file_upload_progress(corr);
        }
        self.remove_multipart_stage_persists(client_upload_id);
    }

    pub(crate) fn clear_all_multipart_runtime(&mut self) {
        let mut client_upload_ids = self
            .state
            .multipart_batches
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        client_upload_ids.extend(
            self.state
                .pending_media_ops
                .values()
                .filter_map(|operation| {
                    let id = multipart_operation_id(operation);
                    (!id.is_empty()).then_some(id)
                }),
        );
        client_upload_ids.extend(self.state.pending_media_stage_persists.values().filter_map(
            |operation| {
                let id = multipart_operation_id(operation);
                (!id.is_empty()).then_some(id)
            },
        ));
        client_upload_ids.extend(self.state.multipart_draining.keys().cloned());
        for client_upload_id in client_upload_ids {
            self.clear_multipart_runtime(&client_upload_id);
        }
    }

    fn handle_too_long_reload_reply(
        &mut self,
        channel_id: ChannelId,
        reset_to: crate::state::Seq,
        trigger: crate::state::SyncTrigger,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let raw_body = match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw) => raw,
            Err(e) => {
                tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?e,
                    "too_long getLatestPost reply envelope decode failed"
                );
                self.clear_too_long_recovery(channel_id);
                self.fail_hydration_for_channel(
                    channel_id,
                    &format!("too_long reload response invalid: {e}"),
                    out,
                );
                return Ok(());
            }
        };
        let mut posts = match extract_latest_posts(raw_body.as_ref()) {
            Ok(posts) => posts,
            Err(error) => {
                self.clear_too_long_recovery(channel_id);
                self.fail_hydration_for_channel(channel_id, &error.to_string(), out);
                return Ok(());
            }
        };
        posts.sort_by_key(|post| {
            post_event_seq(post).unwrap_or_else(|| {
                post.get("createAt")
                    .or_else(|| post.get("create_at"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(reset_to.0)
            })
        });

        let auth_user_id = self.config.auth_user_id.as_str();
        let mut ops = Vec::new();
        let mut channel_updates = Vec::new();
        for post in posts {
            let mut fields = crate::ws::parser::extract_post_fields(&post);
            // 只接受本频道的有效权威帖子；畸形窗口不能被当作空窗口并推进cursor。
            if fields.id.is_empty()
                || (!fields.channel_id.is_empty() && fields.channel_id != channel_id.as_str())
            {
                self.clear_too_long_recovery(channel_id);
                self.fail_hydration_for_channel(
                    channel_id,
                    "too_long latest window post identity invalid",
                    out,
                );
                return Ok(());
            }
            if fields.channel_id.is_empty() {
                fields.channel_id = channel_id.as_str().to_string();
            }
            let seq = post_event_seq(&post).unwrap_or(reset_to.0);
            let msg_id = post
                .get("id")
                .or_else(|| post.get("postId"))
                .or_else(|| post.get("post_id"))
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(fields.id.as_str())
                .to_string();
            let update =
                crate::channel_write::post_updates_from_fields(channel_id, &fields, auth_user_id);
            if !update.visible {
                continue;
            }
            let ev = crate::sync_session::EventEnvelope::new(
                channel_id,
                crate::state::Seq(seq),
                crate::sync_session::EventKind::PostUpsert,
                fields.clone(),
            )
            .with_msg_id(Some(msg_id.clone()));
            // too-long HTTP 窗口可能晚于在线 type=6 回执，按 sync 策略禁止空 read_bits 倒退。
            ops.push(crate::channel::event_to_sync_upsert_op(&ev));
            // hydration 的未读已由 increment authority 落库；历史窗口不再按消息逐条累加。
            if trigger != crate::state::SyncTrigger::Hydration {
                channel_updates.push(crate::channel_update::PendingChannelUpdate::new(
                    channel_id,
                    seq,
                    msg_id.as_str(),
                    &fields,
                    &update,
                    "too_long_reload",
                ));
            }
        }

        for pending in &channel_updates {
            ops.push(crate::acl::to_effect::bump_channel_unread_op(
                &pending.update,
            ));
        }
        // 权威窗口与 cursor 共用同一个写屏障，空窗口也必须如此；重拉失败时旧行与旧 cursor 均不变。
        ops.push(crate::acl::to_effect::advance_cursor_op(
            channel_id, reset_to,
        ));

        let persist_corr = self.alloc_corr_internal();
        let emit_channel_updates = channel_updates
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        out.push(helix_core::Effect::PersistAtomic {
            corr: persist_corr,
            ops,
        });
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::TooLongReloadPersist {
                channel_id,
                reset_to,
                channel_updates: emit_channel_updates,
            },
        );
        if let Some(channel) = self.state.channels.get_mut(&channel_id) {
            channel.inflight_sync = Some(crate::state::InflightSync(persist_corr));
        }
        Ok(())
    }

    /// 释放覆盖恢复的单频道守卫；HTTP sync 的全局窗口已在原响应到达时释放。
    fn clear_too_long_recovery(&mut self, channel_id: ChannelId) {
        if let Some(channel) = self.state.channels.get_mut(&channel_id) {
            channel.inflight_sync = None;
        }
    }

    fn handle_media_port_reply(
        &mut self,
        pending: crate::send::upload_props::PendingMediaOp,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::send::upload_props::{FailedMediaOp, PendingMediaPrepare, PendingMediaPut};
        use helix_core::tick::PortOutcome;

        match pending {
            PendingMediaOp::Prepare(prepare) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_prepare_reply(
                        reply.0.as_ref(),
                        &prepare.input,
                        &prepare.target,
                    ) {
                        Ok(prepared) => {
                            let put = PendingMediaPut {
                                temporary_id: prepare.temporary_id,
                                channel_id: prepare.channel_id,
                                target: prepare.target,
                                input: prepare.input,
                                prepared,
                            };
                            let persist_corr = self.alloc_corr_internal();
                            out.push(helix_core::Effect::Persist {
                                corr: persist_corr,
                                ops: vec![crate::send::upload_props::durable_upsert_op(
                                    &PendingMediaOp::Put(put.clone()),
                                )?],
                            });
                            self.state
                                .pending_media_stage_persists
                                .insert(persist_corr, PendingMediaOp::Put(put));
                        }
                        Err(reason) => {
                            self.fail_media_operation(
                                FailedMediaOp::Prepare(prepare),
                                "prepare_reply",
                                &reason,
                                out,
                            )?;
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    self.fail_media_operation(
                        FailedMediaOp::Prepare(prepare),
                        "prepare_request",
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            },
            PendingMediaOp::Put(put) => match outcome {
                PortOutcome::Ok(_) => self.persist_media_complete_stage(put, out)?,
                PortOutcome::Err(error) => {
                    self.fail_media_operation(
                        FailedMediaOp::Prepare(PendingMediaPrepare {
                            temporary_id: put.temporary_id,
                            channel_id: put.channel_id,
                            target: put.target,
                            input: put.input,
                        }),
                        "put",
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            },
            // Complete 不再发 Java HTTP；该分支仅兼容旧内存路由，直接进入本地终态提交。
            PendingMediaOp::Complete(put) => {
                if matches!(outcome, PortOutcome::Ok(_)) {
                    self.finalize_or_queue_media_completion(put, out)?;
                } else if let PortOutcome::Err(error) = outcome {
                    self.fail_media_completion_sequence(
                        put,
                        "complete_route",
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            }
            PendingMediaOp::MultipartInit(prepare) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_multipart_init_reply(
                        reply.0.as_ref(),
                        &prepare,
                    ) {
                        Ok(session) => {
                            let operation = PendingMediaOp::MultipartParts(session);
                            self.persist_multipart_stage(operation, out)?;
                        }
                        Err(reason) => self.fail_media_operation(
                            FailedMediaOp::Multipart(prepare),
                            "multipart_init_reply",
                            &reason,
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.retry_multipart_on_port_error(
                    prepare,
                    error,
                    "multipart_init_request",
                    out,
                )?,
            },
            PendingMediaOp::MultipartParts(mut session) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_multipart_parts_reply(
                        reply.0.as_ref(),
                        &session,
                    ) {
                        Ok((status, parts)) => {
                            // Java/OSS list is authoritative after restart and after any
                            // lost response; replace local facts rather than trusting them.
                            session.status = status;
                            session.parts = parts;
                            let mut present = [false; 128];
                            for part in &session.parts {
                                // `parse_multipart_parts_reply` already bounds and validates
                                // part numbers. Keep this lookup O(P) even if a future parser
                                // adds another response source.
                                present[part.part_number as usize - 1] = true;
                            }
                            let missing = (1..=session.input.part_count())
                                .filter(|number| !present[*number as usize - 1])
                                .collect::<Vec<_>>();
                            if session.status == "completed" || missing.is_empty() {
                                self.persist_multipart_stage(
                                    PendingMediaOp::MultipartComplete(session),
                                    out,
                                )?;
                            } else {
                                self.persist_multipart_stage(
                                    PendingMediaOp::MultipartSign(PendingMultipartSign {
                                        session,
                                        missing,
                                    }),
                                    out,
                                )?;
                            }
                        }
                        Err(reason) => self.fail_multipart_session(
                            session,
                            "multipart_parts_reply",
                            &reason,
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.retry_multipart_on_port_error(
                    multipart_prepare_from_session(&session),
                    error,
                    "multipart_parts_request",
                    out,
                )?,
            },
            PendingMediaOp::MultipartSign(sign) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_multipart_sign_reply(
                        reply.0.as_ref(),
                        &sign.session,
                        &sign.missing,
                    ) {
                        Ok(signed) => self.start_multipart_batch(sign.session, signed, out)?,
                        Err(reason) => self.fail_multipart_session(
                            sign.session,
                            "multipart_sign_reply",
                            &reason,
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.retry_multipart_on_port_error(
                    multipart_prepare_from_session(&sign.session),
                    error,
                    "multipart_sign_request",
                    out,
                )?,
            },
            PendingMediaOp::MultipartPut(put) => match outcome {
                PortOutcome::Ok(reply) => {
                    let success =
                        crate::send::upload_props::parse_upload_success_reply(reply.0.as_ref())
                            .map_err(|reason| {
                                ImError::Parse(format!("multipart UploadFile: {reason}"))
                            });
                    match success {
                        Ok(success) => {
                            let Some(etag) = success.etag.filter(|etag| !etag.trim().is_empty())
                            else {
                                self.fail_multipart_session(
                                    put.session,
                                    "multipart_upload_etag",
                                    "multipart UploadFile requires non-empty ETag",
                                    out,
                                )?;
                                return Ok(());
                            };
                            let client_upload_id = put.session.client_upload_id.clone();
                            let Some(batch_session) = self
                                .state
                                .multipart_batches
                                .get(&client_upload_id)
                                .map(|batch| batch.session.clone())
                            else {
                                // A failed batch has no active runtime, but its sibling
                                // UploadFile routes remain until this late response. This
                                // reply only releases the drain barrier; it can never resume
                                // business success or start another upload.
                                return self
                                    .finish_multipart_drain_if_ready(&client_upload_id, out);
                            };
                            if success.object_key != batch_session.object_key {
                                self.fail_multipart_session(
                                    batch_session,
                                    "multipart_upload_object_key",
                                    "multipart UploadFile objectKey mismatch",
                                    out,
                                )?;
                                return Ok(());
                            }
                            let (checkpoint, session, duplicate_conflict) = {
                                let Some(batch) =
                                    self.state.multipart_batches.get_mut(&client_upload_id)
                                else {
                                    return Ok(());
                                };
                                if let Some(existing) = batch
                                    .session
                                    .parts
                                    .iter()
                                    .find(|part| part.part_number == put.part.part_number)
                                {
                                    (None, batch.session.clone(), existing.etag != etag)
                                } else {
                                    let size = batch
                                        .session
                                        .input
                                        .part_size(put.part.part_number)
                                        .ok_or_else(|| {
                                            ImError::Parse(
                                                "multipart part out of range".to_string(),
                                            )
                                        })?;
                                    let part = crate::send::upload_props::MultipartPart {
                                        part_number: put.part.part_number,
                                        size,
                                        etag,
                                    };
                                    batch.confirmed_parts[put.part.part_number as usize - 1] = true;
                                    batch.session.parts.push(part.clone());
                                    batch.session.parts.sort_by_key(|part| part.part_number);
                                    (Some(part), batch.session.clone(), false)
                                }
                            };
                            if duplicate_conflict {
                                self.fail_multipart_session(
                                    session,
                                    "multipart_upload_duplicate",
                                    "duplicate multipart part returned a different ETag",
                                    out,
                                )?;
                                return Ok(());
                            }
                            self.persist_multipart_stage(
                                PendingMediaOp::MultipartPut(PendingMultipartPut {
                                    session,
                                    part: put.part,
                                    remaining: Vec::new(),
                                    checkpoint,
                                }),
                                out,
                            )?;
                        }
                        Err(error) => self.fail_multipart_session(
                            put.session,
                            "multipart_upload_reply",
                            &error.to_string(),
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.retry_multipart_on_port_error(
                    multipart_prepare_from_session(&put.session),
                    error,
                    "multipart_upload_request",
                    out,
                )?,
            },
            PendingMediaOp::MultipartComplete(session) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_multipart_complete_reply(
                        reply.0.as_ref(),
                        &session,
                    ) {
                        Ok(prepared) => {
                            self.finalize_multipart_completion(session, prepared, out)?
                        }
                        Err(reason) => self.fail_multipart_session(
                            session,
                            "multipart_complete_reply",
                            &reason,
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.retry_multipart_on_port_error(
                    multipart_prepare_from_session(&session),
                    error,
                    "multipart_complete_request",
                    out,
                )?,
            },
        }
        Ok(())
    }

    fn persist_multipart_stage(
        &mut self,
        operation: crate::send::upload_props::PendingMediaOp,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let persist_corr = self.alloc_corr_internal();
        let mut ops = vec![crate::send::upload_props::durable_upsert_op(&operation)?];
        let session = multipart_session_from_operation(&operation);
        ops.push(crate::send::upload_props::durable_multipart_session_upsert_op(&session));
        match &operation {
            crate::send::upload_props::PendingMediaOp::MultipartPut(put) => {
                if let Some(part) = put.checkpoint.as_ref() {
                    ops.push(crate::send::upload_props::durable_multipart_part_upsert_op(
                        &session, part,
                    ));
                }
            }
            // A Java parts response is the recovery boundary and may replace
            // the local list in one bounded batch. Normal UploadFile replies
            // above use one sparse row per confirmed ETag instead.
            crate::send::upload_props::PendingMediaOp::MultipartParts(_) => {
                ops.push(crate::send::upload_props::durable_multipart_parts_delete_op(&session));
                if let Some(parts) =
                    crate::send::upload_props::durable_multipart_parts_upsert_op(&session)
                {
                    ops.push(parts);
                }
            }
            _ => {}
        }
        out.push(helix_core::Effect::PersistAtomic {
            corr: persist_corr,
            ops,
        });
        self.state
            .pending_media_stage_persists
            .insert(persist_corr, operation);
        Ok(())
    }

    fn emit_multipart_parts(
        &mut self,
        session: MultipartSession,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let req = crate::send::upload_props::multipart_parts_request(
            &self.config.default_api_base_url,
            &session,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        self.state
            .pending_media_ops
            .insert(corr, PendingMediaOp::MultipartParts(session));
        out.push(helix_core::Effect::Http { corr, req });
        Ok(())
    }

    fn emit_multipart_sign(
        &mut self,
        mut sign: PendingMultipartSign,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // Java signs at most eight parts per request. Keep the authoritative
        // missing set in the durable stage, but expose only one bounded batch;
        // after that batch completes, a fresh parts reconciliation discovers
        // the remaining gaps before the next sign request.
        sign.missing.truncate(8);
        let corr = self.alloc_corr_internal();
        let req = crate::send::upload_props::multipart_sign_request(
            &self.config.default_api_base_url,
            &sign.session,
            &sign.missing,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        self.state
            .pending_media_ops
            .insert(corr, PendingMediaOp::MultipartSign(sign));
        out.push(helix_core::Effect::Http { corr, req });
        Ok(())
    }

    fn start_multipart_batch(
        &mut self,
        session: MultipartSession,
        signed: Vec<crate::send::upload_props::MultipartSignedPart>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let client_upload_id = session.client_upload_id.clone();
        let mut confirmed_parts = [false; 128];
        for part in &session.parts {
            confirmed_parts[part.part_number as usize - 1] = true;
        }
        self.state.multipart_batches.insert(
            client_upload_id.clone(),
            MultipartBatch {
                session,
                signed: signed.into_iter().collect(),
                inflight: std::collections::HashSet::new(),
                confirmed_parts,
                progress_bytes: std::collections::HashMap::new(),
            },
        );
        self.emit_multipart_batch_slots(&client_upload_id, out)
    }

    /// Fill up to three transport slots from one sign response. The session
    /// copy in each corr is only a routing snapshot; durable/ordering facts
    /// live in `multipart_batches` and are advanced after checkpoint PersistOk.
    fn emit_multipart_batch_slots(
        &mut self,
        client_upload_id: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut slots = Vec::new();
        {
            let Some(batch) = self.state.multipart_batches.get_mut(client_upload_id) else {
                return Ok(());
            };
            while batch.inflight.len() < MULTIPART_MAX_INFLIGHT {
                let Some(part) = batch.signed.pop_front() else {
                    break;
                };
                batch.inflight.insert(part.part_number);
                slots.push((batch.session.clone(), part));
            }
        }
        for (session, part) in slots {
            if let Err(error) = self.emit_multipart_upload(session, part, Vec::new(), out) {
                self.clear_multipart_runtime(client_upload_id);
                return Err(error);
            }
        }
        Ok(())
    }

    fn emit_multipart_upload(
        &mut self,
        session: MultipartSession,
        part: crate::send::upload_props::MultipartSignedPart,
        remaining: Vec<crate::send::upload_props::MultipartSignedPart>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let req = crate::send::upload_props::multipart_upload_request(
            &session.input,
            &session,
            &part,
            &session.target,
        )?;
        let corr = self.alloc_corr_internal();
        self.register_multipart_upload_progress(
            corr,
            session.temporary_id.clone(),
            session.channel_id,
            &session.target,
            part.part_number,
        );
        self.state.pending_media_ops.insert(
            corr,
            PendingMediaOp::MultipartPut(PendingMultipartPut {
                session,
                part,
                remaining,
                checkpoint: None,
            }),
        );
        out.push(helix_core::Effect::UploadFile { corr, req });
        Ok(())
    }

    fn continue_multipart_batch(
        &mut self,
        completed: PendingMultipartPut,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let client_upload_id = completed.session.client_upload_id.clone();
        let mut next = None;
        let mut requery = None;
        if let Some(batch) = self.state.multipart_batches.get_mut(&client_upload_id) {
            batch.inflight.remove(&completed.part.part_number);
            if batch.inflight.is_empty() && batch.signed.is_empty() {
                requery = Some(batch.session.clone());
            } else if batch.inflight.len() < MULTIPART_MAX_INFLIGHT {
                if let Some(part) = batch.signed.pop_front() {
                    batch.inflight.insert(part.part_number);
                    next = Some((batch.session.clone(), part));
                }
            }
        }
        if let Some((session, part)) = next {
            // The previous ETag PersistAtomic has ACKed. Persist the next
            // in-memory routing stage before exposing another signed URL.
            return self.persist_multipart_stage(
                PendingMediaOp::MultipartPut(PendingMultipartPut {
                    session,
                    part,
                    remaining: Vec::new(),
                    checkpoint: None,
                }),
                out,
            );
        }
        if let Some(session) = requery {
            self.state.multipart_batches.remove(&client_upload_id);
            // The final local checkpoint is durable; a fresh parts query is
            // still required before Java complete, including after a lost
            // UploadFile response or completed-session recovery.
            return self.persist_multipart_stage(PendingMediaOp::MultipartParts(session), out);
        }
        Ok(())
    }

    fn emit_multipart_complete(
        &mut self,
        session: MultipartSession,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let req = crate::send::upload_props::multipart_complete_request(
            &self.config.default_api_base_url,
            &session,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        self.state
            .pending_media_ops
            .insert(corr, PendingMediaOp::MultipartComplete(session));
        out.push(helix_core::Effect::Http { corr, req });
        Ok(())
    }

    fn fail_multipart_session(
        &mut self,
        session: MultipartSession,
        hop: &'static str,
        error: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let client_upload_id = session.client_upload_id.clone();
        self.begin_multipart_drain(
            client_upload_id,
            MultipartDrain {
                pending: crate::send::upload_props::PendingMediaPrepare {
                    temporary_id: session.temporary_id,
                    channel_id: session.channel_id,
                    target: session.target,
                    input: session.input,
                },
                retry_after_ms: None,
                hop,
                error: error.to_string(),
            },
            out,
        )
    }

    /// 网络、超时和 5xx 只重试有限次数；每次重新 init 后再 parts 对账，
    /// 因而不会把本地旧 signed URL 或本地 parts 集合当成远端事实。
    fn retry_multipart_on_port_error(
        &mut self,
        pending: crate::send::upload_props::PendingMediaPrepare,
        error: &helix_core::tick::PortError,
        hop: &'static str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let client_upload_id =
            crate::send::upload_props::client_upload_id(&pending.temporary_id.0, &pending.target);
        if self
            .state
            .multipart_draining
            .contains_key(&client_upload_id)
        {
            return self.finish_multipart_drain_if_ready(&client_upload_id, out);
        }
        let retryable = matches!(
            error,
            helix_core::tick::PortError::Timeout
                | helix_core::tick::PortError::Network
                | helix_core::tick::PortError::Http(408 | 500..=599)
        ) || matches!(
            (hop, error),
            (
                "multipart_upload_request",
                helix_core::tick::PortError::Http(403)
            )
        );
        let key = (pending.temporary_id.clone(), pending.target.clone());
        let retry_after_ms = if retryable {
            let attempt = {
                let attempts = self
                    .state
                    .multipart_retry_attempts
                    .entry(key.clone())
                    .or_default();
                if *attempts < 3 {
                    *attempts += 1;
                    Some(*attempts)
                } else {
                    None
                }
            };
            if let Some(attempt) = attempt {
                Some(100_u64.saturating_mul(1_u64 << (attempt - 1)))
            } else {
                None
            }
        } else {
            None
        };
        self.begin_multipart_drain(
            client_upload_id,
            MultipartDrain {
                pending,
                retry_after_ms,
                hop,
                error: format!("{error:?}"),
            },
            out,
        )
    }

    fn finalize_multipart_completion(
        &mut self,
        session: MultipartSession,
        prepared: crate::send::upload_props::PreparedMedia,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.finalize_or_queue_media_completion(
            crate::send::upload_props::PendingMediaPut {
                temporary_id: session.temporary_id,
                channel_id: session.channel_id,
                target: session.target,
                input: session.input,
                prepared,
            },
            out,
        )
    }

    fn persist_media_complete_stage(
        &mut self,
        put: crate::send::upload_props::PendingMediaPut,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let operation = crate::send::upload_props::PendingMediaOp::Complete(put);
        let persist_corr = self.alloc_corr_internal();
        out.push(helix_core::Effect::Persist {
            corr: persist_corr,
            ops: vec![crate::send::upload_props::durable_upsert_op(&operation)?],
        });
        self.state
            .pending_media_stage_persists
            .insert(persist_corr, operation);
        Ok(())
    }

    fn handle_media_stage_persist_reply(
        &mut self,
        pending: crate::send::upload_props::PendingMediaOp,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::send::upload_props::{FailedMediaOp, PendingMediaPrepare};
        use helix_core::tick::PortOutcome;

        let multipart_id = multipart_operation_id(&pending);
        if !multipart_id.is_empty() && self.state.multipart_draining.contains_key(&multipart_id) {
            // The stage was already issued before the batch failed. Its
            // PersistAtomic reply is only a drain barrier; never expose the
            // signed URL or checkpoint continuation after failure.
            return self.finish_multipart_drain_if_ready(&multipart_id, out);
        }

        match outcome {
            PortOutcome::Ok(_) => match pending {
                PendingMediaOp::Prepare(prepare) => self.emit_media_prepare(
                    prepare.channel_id,
                    prepare.temporary_id,
                    prepare.target,
                    prepare.input,
                    out,
                )?,
                PendingMediaOp::Put(put) => {
                    let req = crate::send::upload_props::media_upload_request(
                        &put.input,
                        &put.prepared,
                        &put.target,
                    )?;
                    let corr = self.alloc_corr_internal();
                    self.register_file_upload_progress(
                        corr,
                        put.temporary_id.clone(),
                        put.channel_id,
                        &put.target,
                    );
                    self.state
                        .pending_media_ops
                        .insert(corr, PendingMediaOp::Put(put));
                    out.push(helix_core::Effect::UploadFile { corr, req });
                }
                PendingMediaOp::Complete(put) => {
                    self.finalize_or_queue_media_completion(put, out)?;
                }
                PendingMediaOp::MultipartInit(prepare) => self.emit_multipart_init(
                    prepare.channel_id,
                    prepare.temporary_id,
                    prepare.target,
                    prepare.input,
                    out,
                )?,
                PendingMediaOp::MultipartParts(session) => {
                    self.emit_multipart_parts(session, out)?;
                }
                PendingMediaOp::MultipartSign(sign) => {
                    self.emit_multipart_sign(sign, out)?;
                }
                PendingMediaOp::MultipartPut(put) => {
                    if put.checkpoint.is_none() {
                        // This is a next-slot routing stage. Its part number
                        // was reserved in the batch before PersistAtomic;
                        // expose the signed UploadFile only after that ACK.
                        self.emit_multipart_upload(put.session, put.part, put.remaining, out)?;
                    } else {
                        self.continue_multipart_batch(put, out)?;
                    }
                }
                PendingMediaOp::MultipartComplete(session) => {
                    self.emit_multipart_complete(session, out)?;
                }
            },
            PortOutcome::Err(error) => {
                let fallback = match pending {
                    PendingMediaOp::Prepare(prepare) => FailedMediaOp::Prepare(prepare),
                    PendingMediaOp::Put(put) => FailedMediaOp::Prepare(PendingMediaPrepare {
                        temporary_id: put.temporary_id,
                        channel_id: put.channel_id,
                        target: put.target,
                        input: put.input,
                    }),
                    PendingMediaOp::Complete(put) => FailedMediaOp::Complete(put),
                    PendingMediaOp::MultipartInit(prepare) => FailedMediaOp::Multipart(prepare),
                    PendingMediaOp::MultipartParts(session)
                    | PendingMediaOp::MultipartComplete(session) => {
                        FailedMediaOp::Multipart(PendingMediaPrepare {
                            temporary_id: session.temporary_id,
                            channel_id: session.channel_id,
                            target: session.target,
                            input: session.input,
                        })
                    }
                    PendingMediaOp::MultipartSign(sign) => {
                        FailedMediaOp::Multipart(PendingMediaPrepare {
                            temporary_id: sign.session.temporary_id,
                            channel_id: sign.session.channel_id,
                            target: sign.session.target,
                            input: sign.session.input,
                        })
                    }
                    PendingMediaOp::MultipartPut(put) => {
                        FailedMediaOp::Multipart(PendingMediaPrepare {
                            temporary_id: put.session.temporary_id,
                            channel_id: put.session.channel_id,
                            target: put.session.target,
                            input: put.session.input,
                        })
                    }
                };
                if !multipart_id.is_empty() {
                    match fallback {
                        FailedMediaOp::Multipart(prepare) => {
                            self.begin_multipart_drain(
                                multipart_id,
                                MultipartDrain {
                                    pending: prepare,
                                    retry_after_ms: None,
                                    hop: "stage_persist",
                                    error: format!("{error:?}"),
                                },
                                out,
                            )?;
                        }
                        other => {
                            self.fail_media_operation(
                                other,
                                "stage_persist",
                                &format!("{error:?}"),
                                out,
                            )?;
                        }
                    }
                } else {
                    self.fail_media_operation(
                        fallback,
                        "stage_persist",
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// 串行化同一消息的本地 Complete，避免多个附件并发覆盖整段 props。
    pub(crate) fn finalize_or_queue_media_completion(
        &mut self,
        put: crate::send::upload_props::PendingMediaPut,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let temporary_id = put.temporary_id.clone();
        if self.state.media_completion_inflight.contains(&temporary_id) {
            self.state
                .queued_media_completions
                .entry(temporary_id)
                .or_default()
                .push_back(put);
            return Ok(());
        }
        self.state.media_completion_inflight.insert(temporary_id);
        self.persist_media_complete(put, out)
    }

    // 统一回收失败的本地 Complete 串行槽，并把排队附件恢复为可重试状态。
    fn fail_media_completion_sequence(
        &mut self,
        failed: crate::send::upload_props::PendingMediaPut,
        hop: &'static str,
        error: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let temporary_id = failed.temporary_id.clone();
        self.state.media_completion_inflight.remove(&temporary_id);
        if let Some(queued) = self.state.queued_media_completions.remove(&temporary_id) {
            for pending in queued {
                self.state.failed_media_ops.insert(
                    (temporary_id.clone(), pending.target.clone()),
                    failed_media_op_for_put(&pending),
                );
            }
        }
        self.fail_media_operation(failed_media_op_for_put(&failed), hop, error, out)
    }

    // 释放当前串行槽并取出同一消息的下一个待完成附件。
    fn take_next_media_completion(
        &mut self,
        temporary_id: &crate::state::TemporaryId,
    ) -> Option<crate::send::upload_props::PendingMediaPut> {
        let next = self
            .state
            .queued_media_completions
            .get_mut(temporary_id)
            .and_then(std::collections::VecDeque::pop_front);
        if self
            .state
            .queued_media_completions
            .get(temporary_id)
            .is_some_and(std::collections::VecDeque::is_empty)
        {
            self.state.queued_media_completions.remove(temporary_id);
        }
        self.state.media_completion_inflight.remove(temporary_id);
        next
    }

    fn fail_media_operation(
        &mut self,
        failed: crate::send::upload_props::FailedMediaOp,
        hop: &'static str,
        error: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let pending = crate::send::upload_props::PendingUpload {
            temporary_id: failed.temporary_id().clone(),
            target: failed.target().clone(),
        };
        tracing::warn!(
            tmp_id = pending.temporary_id.0.as_str(),
            target = ?pending.target,
            hop,
            reason = error,
            "media upload operation failed"
        );
        self.state
            .media_retry_inflight
            .remove(&pending.temporary_id);
        if matches!(
            &failed,
            crate::send::upload_props::FailedMediaOp::Multipart(_)
        ) {
            self.state
                .multipart_retry_attempts
                .remove(&(pending.temporary_id.clone(), pending.target.clone()));
        }
        self.state.failed_media_ops.insert(
            (pending.temporary_id.clone(), pending.target.clone()),
            failed,
        );
        self.mark_upload_failed(pending, error, out)
    }

    // 跳过已完成 file id，否则启动消息引用与 journal 删除的原子提交。
    fn persist_media_complete(
        &mut self,
        pending: crate::send::upload_props::PendingMediaPut,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self
            .state
            .completed_media
            .contains(&pending.prepared.file_id)
        {
            let temporary_id = pending.temporary_id.clone();
            if let Some(next) = self.take_next_media_completion(&temporary_id) {
                self.finalize_or_queue_media_completion(next, out)?;
            }
            return Ok(());
        }
        self.start_media_completion_persist(pending, out)
    }

    // 先原子提交稳定 bucket/uri 与 journal 删除，ACK 前不推进 posts/create。
    fn start_media_completion_persist(
        &mut self,
        pending: crate::send::upload_props::PendingMediaPut,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let props_snapshot = self
            .state
            .pending_sends
            .get(&pending.temporary_id)
            .and_then(|send| send.body.as_ref())
            .map(|body| body.get("props").cloned());
        let Some(props_snapshot) = props_snapshot else {
            return self.fail_media_completion_sequence(
                pending,
                "complete_state",
                "missing pending send body",
                out,
            );
        };
        let Some(mut props_after) = props_snapshot else {
            return self.fail_media_completion_sequence(
                pending,
                "complete_state",
                "missing media props",
                out,
            );
        };
        if let Err(error) =
            crate::send::upload_props::mark_media_complete(&mut props_after, &pending)
        {
            return self.fail_media_completion_sequence(
                pending,
                "complete_state",
                &error.to_string(),
                out,
            );
        }
        let props_json = serde_json::to_string(&props_after)
            .map_err(|error| ImError::Serialize(error.to_string()))?;
        let persist_corr = self.alloc_corr_internal();
        let is_final_media = self
            .state
            .pending_sends
            .get(&pending.temporary_id)
            .is_some_and(|send| send.remaining_uploads == 1 && !send.upload_failed);
        let is_file = matches!(
            &pending.target,
            crate::send::upload_props::UploadTarget::File
        );
        let message_op = if is_final_media {
            // 最后一个附件的可信引用与 durable retry marker 同事务提交。内存态仍保持
            // Sending，ACK 后立即发 posts/create；若进程死在 ACK→HTTP 窗口，重启后的
            // completed props + unsend 行可由 im_retry_send 安全重放同一 temporaryId。
            crate::send::upload_props::media_send_state_persist_op(
                &pending.temporary_id,
                props_json,
                "unsend",
                is_file.then_some(100),
            )
        } else {
            crate::pending_send::props_persist_op(&pending.temporary_id, props_json)
        };
        let mut ops = vec![
            message_op,
            crate::send::upload_props::durable_delete_op(&pending.temporary_id, &pending.target),
        ];
        if pending.prepared.method == "MULTIPART" {
            ops.extend(crate::send::upload_props::durable_multipart_delete_ops(
                &multipart_session_from_put(&pending),
            ));
        }
        if is_file && !is_final_media {
            ops.push(crate::pending_send::upload_progress_persist_op(
                &pending.temporary_id,
                100,
            ));
        }
        out.push(helix_core::Effect::PersistAtomic {
            corr: persist_corr,
            ops,
        });
        self.track_media_write(&pending.temporary_id, persist_corr);
        self.state.pending_media_completion_persists.insert(
            persist_corr,
            crate::send::upload_props::PendingMediaCompletionCommit {
                pending,
                props_after,
            },
        );
        Ok(())
    }

    // 在本地 Complete 原子提交成功后推进内存态、下一附件或最终 posts/create。
    fn handle_media_completion_persist_reply(
        &mut self,
        commit: crate::send::upload_props::PendingMediaCompletionCommit,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        match outcome {
            PortOutcome::Ok(_) => {
                let crate::send::upload_props::PendingMediaCompletionCommit {
                    pending,
                    props_after,
                } = commit;
                let crate::send::upload_props::PendingMediaPut {
                    temporary_id,
                    channel_id,
                    target,
                    prepared,
                    ..
                } = pending;
                let mut ready_body = None;
                if let Some(send) = self.state.pending_sends.get_mut(&temporary_id) {
                    if let Some(body) = send.body.as_mut() {
                        body["props"] = props_after;
                    }
                    if send.remaining_uploads > 0 {
                        send.remaining_uploads -= 1;
                    }
                    if send.remaining_uploads == 0 && !send.upload_failed {
                        ready_body = send.body.clone();
                    }
                }
                self.state.completed_media.insert(prepared.file_id);
                self.state
                    .multipart_retry_attempts
                    .remove(&(temporary_id.clone(), target));

                let next_completion = self.take_next_media_completion(&temporary_id);
                if let Some(pending) = next_completion {
                    self.finalize_or_queue_media_completion(pending, out)?;
                } else if let Some(body) = ready_body {
                    self.state.media_retry_inflight.remove(&temporary_id);
                    self.emit_posts_create_http(channel_id, temporary_id, &body, out)?;
                }
            }
            PortOutcome::Err(error) => {
                self.fail_media_completion_sequence(
                    commit.pending,
                    "complete_persist",
                    &format!("{error:?}"),
                    out,
                )?;
            }
        }
        Ok(())
    }

    fn handle_upload_port_reply(
        &mut self,
        pending_upload: crate::send::upload_props::PendingUpload,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        match outcome {
            PortOutcome::Ok(reply) => {
                match crate::send::upload_props::parse_upload_success_reply(reply.0.as_ref()) {
                    Ok(success) => {
                        self.mark_upload_success(pending_upload, &success, out)?;
                    }
                    Err(reason) => {
                        tracing::warn!(
                            tmp_id = pending_upload.temporary_id.0.as_str(),
                            error = reason.as_str(),
                            "upload reply malformed, downgrading to upload failed"
                        );
                        self.mark_upload_failed(pending_upload, reason.as_str(), out)?;
                    }
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(
                    tmp_id = pending_upload.temporary_id.0.as_str(),
                    error = ?e,
                    "upload failed"
                );
                self.mark_upload_failed(pending_upload, &format!("{e:?}"), out)?;
            }
        }

        Ok(())
    }

    fn mark_upload_success(
        &mut self,
        pending_upload: crate::send::upload_props::PendingUpload,
        success: &crate::send::upload_props::UploadSuccess,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let persist_corr = self.alloc_corr_internal();
        let mut ready_body = None;
        if let Some(ps) = self
            .state
            .pending_sends
            .get_mut(&pending_upload.temporary_id)
        {
            let Some(body) = ps.body.as_mut() else {
                tracing::warn!(
                    tmp_id = pending_upload.temporary_id.0.as_str(),
                    "upload success without cached send body"
                );
                return Ok(());
            };
            let Some(props) = body.get_mut("props") else {
                tracing::warn!(
                    tmp_id = pending_upload.temporary_id.0.as_str(),
                    "upload success without props"
                );
                return Ok(());
            };
            crate::send::upload_props::mark_upload_success(props, &pending_upload.target, success)?;
            let props_json =
                serde_json::to_string(props).map_err(|e| ImError::Serialize(e.to_string()))?;
            let mut ops = vec![crate::pending_send::props_persist_op(
                &pending_upload.temporary_id,
                props_json,
            )];
            if matches!(
                &pending_upload.target,
                crate::send::upload_props::UploadTarget::File
            ) {
                ops.push(crate::pending_send::upload_progress_persist_op(
                    &pending_upload.temporary_id,
                    100,
                ));
            }
            out.push(helix_core::Effect::Persist {
                corr: persist_corr,
                ops,
            });
            if ps.remaining_uploads > 0 {
                ps.remaining_uploads -= 1;
            }
            if ps.remaining_uploads == 0 && !ps.upload_failed {
                ready_body = Some(body.clone());
            }
        }

        if let Some(body) = ready_body {
            let channel_id = body
                .get("channelId")
                .and_then(serde_json::Value::as_str)
                .and_then(ChannelId::from_str)
                .ok_or_else(|| ImError::Parse("upload-gated send missing channelId".to_string()))?;
            self.emit_posts_create_http(channel_id, pending_upload.temporary_id, &body, out)?;
        }

        Ok(())
    }

    fn mark_upload_failed(
        &mut self,
        pending_upload: crate::send::upload_props::PendingUpload,
        error: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let persist_corr = self.alloc_corr_internal();
        let Some(ps) = self
            .state
            .pending_sends
            .get_mut(&pending_upload.temporary_id)
        else {
            return Ok(());
        };
        ps.upload_failed = true;
        ps.status = crate::state::SendStatus::UnSend;
        let crate::pending_send::TimelineReadbackContext {
            window_token,
            causation_id,
        } = ps.timeline_readback.clone();
        let Some(body) = ps.body.as_mut() else {
            tracing::warn!(
                tmp_id = pending_upload.temporary_id.0.as_str(),
                "upload failure without cached send body"
            );
            return Ok(());
        };
        let channel_id = body
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("upload failure missing channelId".to_string()))?;
        let Some(props) = body.get_mut("props") else {
            tracing::warn!(
                tmp_id = pending_upload.temporary_id.0.as_str(),
                "upload failure without props"
            );
            return Ok(());
        };
        crate::send::upload_props::mark_upload_failed(props, &pending_upload.target, error)?;
        let props_json =
            serde_json::to_string(props).map_err(|e| ImError::Serialize(e.to_string()))?;
        let reset_progress = matches!(
            &pending_upload.target,
            crate::send::upload_props::UploadTarget::File
        )
        .then_some(0);
        out.push(helix_core::Effect::Persist {
            corr: persist_corr,
            ops: vec![crate::send::upload_props::media_send_state_persist_op(
                &pending_upload.temporary_id,
                props_json,
                "unsend",
                reset_progress,
            )],
        });
        self.track_media_write(&pending_upload.temporary_id, persist_corr);
        let failed_event = crate::event::post::send_failed_for_identity_with_request_id(
            channel_id.as_str(),
            pending_upload.temporary_id.0.as_str(),
            causation_id.as_deref(),
        )?;
        self.state.corr_map.insert(
            persist_corr,
            crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token,
                causation_id,
            },
        );
        out.push(failed_event.into_effect());
        Ok(())
    }
}

fn failed_media_op_for_put(
    pending: &crate::send::upload_props::PendingMediaPut,
) -> crate::send::upload_props::FailedMediaOp {
    if pending.input.uses_multipart() || pending.prepared.method == "MULTIPART" {
        crate::send::upload_props::FailedMediaOp::Multipart(
            crate::send::upload_props::PendingMediaPrepare {
                temporary_id: pending.temporary_id.clone(),
                channel_id: pending.channel_id,
                target: pending.target.clone(),
                input: pending.input.clone(),
            },
        )
    } else {
        crate::send::upload_props::FailedMediaOp::Complete(pending.clone())
    }
}

fn multipart_session_from_put(
    pending: &crate::send::upload_props::PendingMediaPut,
) -> crate::send::upload_props::MultipartSession {
    crate::send::upload_props::MultipartSession {
        temporary_id: pending.temporary_id.clone(),
        channel_id: pending.channel_id,
        target: pending.target.clone(),
        input: pending.input.clone(),
        client_upload_id: crate::send::upload_props::client_upload_id(
            &pending.temporary_id.0,
            &pending.target,
        ),
        upload_id: String::new(),
        file_id: pending.prepared.file_id.clone(),
        bucket: pending.prepared.bucket.clone(),
        object_key: pending.prepared.object_key.clone(),
        uri: pending.prepared.uri.clone(),
        part_size: crate::send::upload_props::MULTIPART_PART_SIZE_BYTES,
        status: "completed".to_string(),
        parts: Vec::new(),
    }
}

fn extract_latest_posts(raw_body: &[u8]) -> Result<Vec<serde_json::Value>, ImError> {
    let root = serde_json::from_slice::<serde_json::Value>(raw_body)
        .map_err(|error| ImError::Parse(format!("too_long latest window invalid JSON: {error}")))?;
    for candidate in [
        root.pointer("/data/posts"),
        root.pointer("/data/items"),
        root.pointer("/data/list"),
        root.pointer("/data"),
        root.get("posts"),
        root.get("items"),
    ] {
        if let Some(serde_json::Value::Array(items)) = candidate {
            if items.iter().any(|item| !item.is_object()) {
                return Err(ImError::Parse(
                    "too_long latest window contains invalid post".to_string(),
                ));
            }
            return Ok(items
                .iter()
                .filter(|item| item.is_object())
                .cloned()
                .collect());
        }
    }
    Err(ImError::Parse(
        "too_long latest window missing posts array".to_string(),
    ))
}

fn post_event_seq(post: &serde_json::Value) -> Option<u64> {
    post.get("eventSeq")
        .or_else(|| post.get("event_seq"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            post.get("props")
                .and_then(|props| props.get("channel_event_seq"))
                .and_then(serde_json::Value::as_u64)
        })
}
