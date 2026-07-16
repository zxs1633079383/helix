use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::EffectSink;

mod dispatch;
mod dispatch_channel_create;
mod dispatch_core;
mod dispatch_read;

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
        use crate::send::upload_props::{PendingMediaOp, PendingMediaPut, PendingUpload};
        use helix_core::tick::PortOutcome;

        match pending {
            PendingMediaOp::Prepare(prepare) => match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::send::upload_props::parse_prepare_reply(
                        reply.0.as_ref(),
                        &prepare.input,
                        prepare.channel_id.as_str(),
                    ) {
                        Ok(prepared) => {
                            let req = crate::send::upload_props::media_upload_request(
                                &prepare.input,
                                &prepared,
                            )?;
                            let corr = self.alloc_corr_internal();
                            self.state.pending_media_ops.insert(
                                corr,
                                PendingMediaOp::Put(PendingMediaPut {
                                    temporary_id: prepare.temporary_id,
                                    channel_id: prepare.channel_id,
                                    target: prepare.target,
                                    input: prepare.input,
                                    prepared,
                                }),
                            );
                            out.push(helix_core::Effect::UploadFile { corr, req });
                        }
                        Err(reason) => {
                            self.mark_upload_failed(
                                PendingUpload {
                                    temporary_id: prepare.temporary_id,
                                    target: prepare.target,
                                },
                                &reason,
                                out,
                            )?;
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    self.mark_upload_failed(
                        PendingUpload {
                            temporary_id: prepare.temporary_id,
                            target: prepare.target,
                        },
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            },
            PendingMediaOp::Put(put) => match outcome {
                PortOutcome::Ok(_) => {
                    let corr = self.alloc_corr_internal();
                    let req = crate::send::upload_props::complete_upload_request(
                        &self.config.api_base_url,
                        &put.prepared.upload_id,
                        crate::acl::sync_http_effects::session_auth_headers(
                            self.state.connection_id.as_deref(),
                        ),
                    )?;
                    self.state
                        .waiting_media_complete
                        .insert(put.prepared.upload_id.clone(), put.clone());
                    self.state
                        .pending_media_ops
                        .insert(corr, PendingMediaOp::Complete(put));
                    out.push(helix_core::Effect::Http { corr, req });
                }
                PortOutcome::Err(error) => {
                    self.mark_upload_failed(
                        PendingUpload {
                            temporary_id: put.temporary_id,
                            target: put.target,
                        },
                        &format!("{error:?}"),
                        out,
                    )?;
                }
            },
            PendingMediaOp::Complete(put) => {
                self.state
                    .waiting_media_complete
                    .remove(&put.prepared.upload_id);
                match outcome {
                    PortOutcome::Ok(reply) => {
                        match crate::send::upload_props::parse_complete_reply(
                            reply.0.as_ref(),
                            &put,
                        ) {
                            Ok(reference) => {
                                self.persist_media_complete(put, reference, out)?;
                            }
                            Err(reason) => {
                                self.mark_upload_failed(
                                    PendingUpload {
                                        temporary_id: put.temporary_id,
                                        target: put.target,
                                    },
                                    &reason,
                                    out,
                                )?;
                            }
                        }
                    }
                    PortOutcome::Err(error) => {
                        self.mark_upload_failed(
                            PendingUpload {
                                temporary_id: put.temporary_id,
                                target: put.target,
                            },
                            &format!("{error:?}"),
                            out,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn persist_media_complete(
        &mut self,
        pending: crate::send::upload_props::PendingMediaPut,
        reference: crate::send::upload_props::TrustedMediaReference,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let persist_corr = self.alloc_corr_internal();
        let Some(send) = self.state.pending_sends.get_mut(&pending.temporary_id) else {
            return Ok(());
        };
        let Some(body) = send.body.as_mut() else {
            return Ok(());
        };
        let Some(props) = body.get_mut("props") else {
            return Ok(());
        };
        crate::send::upload_props::mark_media_complete(props, &pending.target, &reference)?;
        let props_json =
            serde_json::to_string(props).map_err(|error| ImError::Serialize(error.to_string()))?;
        out.push(helix_core::Effect::Persist {
            corr: persist_corr,
            ops: vec![crate::pending_send::props_persist_op(
                &pending.temporary_id,
                props_json,
            )],
        });
        self.state.completed_media.insert(
            pending.prepared.upload_id,
            crate::send::upload_props::CompletedMedia {
                temporary_id: pending.temporary_id,
                target: pending.target,
                reference,
            },
        );
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
            out.push(helix_core::Effect::Persist {
                corr: persist_corr,
                ops: vec![crate::pending_send::props_persist_op(
                    &pending_upload.temporary_id,
                    props_json,
                )],
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
        if let Some(ps) = self
            .state
            .pending_sends
            .get_mut(&pending_upload.temporary_id)
        {
            ps.upload_failed = true;
            let Some(body) = ps.body.as_mut() else {
                tracing::warn!(
                    tmp_id = pending_upload.temporary_id.0.as_str(),
                    "upload failure without cached send body"
                );
                return Ok(());
            };
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
            out.push(helix_core::Effect::Persist {
                corr: persist_corr,
                ops: vec![crate::pending_send::props_persist_op(
                    &pending_upload.temporary_id,
                    props_json,
                )],
            });
        }
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
