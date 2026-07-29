use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::EffectSink;

mod dispatch;
mod dispatch_channel_create;
mod dispatch_core;
mod dispatch_projection;
mod dispatch_read;
pub(crate) mod message_v3_post;
mod progress;

impl ImModule {
    fn handle_too_long_reload_reply(
        &mut self,
        channel_id: ChannelId,
        reset_to: crate::state::Seq,
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
                return Ok(());
            }
        };
        let mut posts = extract_latest_posts(raw_body.as_ref());
        if posts.is_empty() {
            self.finish_increment_hydration(channel_id, out)?;
            return Ok(());
        }
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
        let mut dialog_patches = Vec::new();
        for post in posts {
            let mut fields = crate::ws::parser::extract_post_fields(&post);
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
            ops.push(crate::channel::event_to_upsert_op(&ev));
            dialog_patches.push(crate::dialog_patch::PendingDialogPatch::new(
                channel_id,
                seq,
                msg_id.as_str(),
                &fields,
                &update,
                "too_long_reload",
            ));
        }

        for pending in &dialog_patches {
            ops.push(crate::acl::to_effect::bump_channel_unread_op(
                &pending.update,
            ));
        }
        if !dialog_patches.is_empty() {
            ops.push(crate::acl::to_effect::get_channel_row_op(channel_id));
        }
        if ops.is_empty() {
            self.finish_increment_hydration(channel_id, out)?;
            return Ok(());
        }

        let persist_corr = self.alloc_corr_internal();
        let emit_dialog_patches = dialog_patches
            .last()
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        out.push(helix_core::Effect::Persist {
            corr: persist_corr,
            ops,
        });
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::TooLongReloadPersist {
                channel_id,
                dialog_patches: emit_dialog_patches,
            },
        );
        Ok(())
    }

    fn handle_media_port_reply(
        &mut self,
        pending: crate::send::upload_props::PendingMediaOp,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::send::upload_props::{
            FailedMediaOp, PendingMediaOp, PendingMediaPrepare, PendingMediaPut,
        };
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
        }
        Ok(())
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
        use crate::send::upload_props::{FailedMediaOp, PendingMediaOp, PendingMediaPrepare};
        use helix_core::tick::PortOutcome;

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
                };
                self.fail_media_operation(fallback, "stage_persist", &format!("{error:?}"), out)?;
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
                    crate::send::upload_props::FailedMediaOp::Complete(pending),
                );
            }
        }
        self.fail_media_operation(
            crate::send::upload_props::FailedMediaOp::Complete(failed),
            hop,
            error,
            out,
        )
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
        self.state.corr_map.insert(
            persist_corr,
            crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token,
                causation_id,
            },
        );
        out.push(crate::acl::to_effect::emit_post_send_failed(
            channel_id.as_str(),
            pending_upload.temporary_id.0.as_str(),
        ));
        Ok(())
    }
}

fn extract_latest_posts(raw_body: &[u8]) -> Vec<serde_json::Value> {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(raw_body) else {
        return Vec::new();
    };
    for candidate in [
        root.pointer("/data/posts"),
        root.pointer("/data/items"),
        root.pointer("/data/list"),
        root.pointer("/data"),
        root.get("posts"),
        root.get("items"),
    ] {
        if let Some(serde_json::Value::Array(items)) = candidate {
            return items
                .iter()
                .filter(|item| item.is_object())
                .cloned()
                .collect();
        }
    }
    Vec::new()
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
