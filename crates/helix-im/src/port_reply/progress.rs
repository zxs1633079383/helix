use crate::error::ImError;
use crate::module::ImModule;
use crate::send::upload_props::UploadTarget;
use crate::state::{ChannelId, FileUploadProgressPersist, FileUploadProgressState, TemporaryId};
use helix_core::effect::FileUploadProgress;
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink};

impl ImModule {
    pub(crate) fn defer_media_put_until_progress_commits(
        &mut self,
        upload_corr: Correlation,
        operation: &crate::send::upload_props::PendingMediaOp,
    ) -> bool {
        let pending = match operation {
            crate::send::upload_props::PendingMediaOp::Put(pending) => pending.clone(),
            // Multipart UploadFile replies carry the required ETag. Do not defer
            // them behind progress persistence, which has no reply payload to save.
            crate::send::upload_props::PendingMediaOp::MultipartPut(_) => return false,
            _ => return false,
        };
        let must_wait = self
            .state
            .file_upload_progress
            .get(&upload_corr)
            .is_some_and(|state| state.inflight_persist.is_some());
        if must_wait {
            self.state
                .media_put_after_progress
                .insert(upload_corr, pending);
        }
        must_wait
    }

    pub(crate) fn register_file_upload_progress(
        &mut self,
        upload_corr: Correlation,
        temporary_id: TemporaryId,
        channel_id: ChannelId,
        target: &UploadTarget,
    ) {
        if matches!(target, UploadTarget::TemplateImage) {
            return;
        }
        let timeline_readback = self
            .state
            .pending_sends
            .get(&temporary_id)
            .map(|pending| pending.timeline_readback.clone())
            .unwrap_or_default();
        self.state.file_upload_progress.insert(
            upload_corr,
            FileUploadProgressState {
                target: target.clone(),
                temporary_id,
                channel_id,
                part_number: None,
                timeline_readback,
                highest_observed_percent: 0,
                committed_percent: 0,
                inflight_persist: None,
                queued_latest: None,
            },
        );
    }

    pub(crate) fn register_multipart_upload_progress(
        &mut self,
        upload_corr: Correlation,
        temporary_id: TemporaryId,
        channel_id: ChannelId,
        target: &UploadTarget,
        part_number: u32,
    ) {
        self.register_file_upload_progress(upload_corr, temporary_id, channel_id, target);
        if let Some(state) = self.state.file_upload_progress.get_mut(&upload_corr) {
            state.part_number = Some(part_number);
        }
    }

    pub(crate) fn handle_file_upload_progress(
        &mut self,
        upload_corr: Correlation,
        progress: FileUploadProgress,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if progress.total_bytes == 0 || progress.completed_bytes > progress.total_bytes {
            return Ok(());
        }
        let raw_percent =
            ((u128::from(progress.completed_bytes) * 100) / u128::from(progress.total_bytes)) as u8;
        let multipart_id = match self.state.pending_media_ops.get(&upload_corr) {
            Some(crate::send::upload_props::PendingMediaOp::MultipartPut(pending)) => Some((
                pending.session.client_upload_id.clone(),
                pending.part.part_number,
            )),
            _ => None,
        };
        let percent = if let Some((client_upload_id, part_number)) = multipart_id {
            if let Some(batch) = self.state.multipart_batches.get_mut(&client_upload_id) {
                let bytes = progress.completed_bytes.min(progress.total_bytes);
                batch
                    .progress_bytes
                    .entry(part_number)
                    .and_modify(|current| *current = (*current).max(bytes))
                    .or_insert(bytes);
            }
            // Aggregate confirmed part bytes plus active range bytes. A local
            // range EOF still caps at 99 until Java complete + PersistOk.
            let aggregate = self
                .state
                .multipart_batches
                .get(&client_upload_id)
                .map(|batch| {
                    let confirmed = batch
                        .session
                        .parts
                        .iter()
                        .map(|part| part.size)
                        .sum::<u64>();
                    let active = batch
                        .progress_bytes
                        .iter()
                        .filter(|(number, _)| {
                            batch
                                .confirmed_parts
                                .get((**number as usize).saturating_sub(1))
                                .is_some_and(|confirmed| !confirmed)
                        })
                        .map(|(number, bytes)| {
                            batch
                                .session
                                .input
                                .part_size(*number)
                                .map_or(0, |size| (*bytes).min(size))
                        })
                        .sum::<u64>();
                    (((u128::from(confirmed.saturating_add(active)) * 100)
                        / u128::from(batch.session.input.size))
                    .min(99)) as u8
                })
                .unwrap_or(raw_percent.min(99));
            (aggregate / 5) * 5
        } else {
            (raw_percent / 5) * 5
        };
        if percent == 0 {
            return Ok(());
        }

        let Some(state) = self.state.file_upload_progress.get_mut(&upload_corr) else {
            return Ok(());
        };
        if percent <= state.highest_observed_percent {
            return Ok(());
        }
        state.highest_observed_percent = percent;
        if state.inflight_persist.is_some() {
            state.queued_latest = Some(percent);
            return Ok(());
        }
        if percent <= state.committed_percent {
            return Ok(());
        }

        state.queued_latest = Some(percent);
        let temporary_id = state.temporary_id.clone();
        self.enqueue_media_progress(&temporary_id, upload_corr);
        self.drain_media_writes(&temporary_id, out)
    }

    pub(crate) fn handle_file_upload_progress_persist_reply(
        &mut self,
        persist_corr: Correlation,
        route: FileUploadProgressPersist,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let (temporary_id, channel_id, timeline_readback, next_percent, committed) = {
            let Some(state) = self.state.file_upload_progress.get_mut(&route.upload_corr) else {
                return Ok(());
            };
            if state.inflight_persist != Some((persist_corr, route.percent)) {
                return Ok(());
            }
            state.inflight_persist = None;
            let committed = matches!(outcome, PortOutcome::Ok(_));
            if committed {
                state.committed_percent = state.committed_percent.max(route.percent);
            }
            let next_percent = state
                .queued_latest
                .take()
                .filter(|percent| *percent > state.committed_percent);
            (
                state.temporary_id.clone(),
                state.channel_id,
                state.timeline_readback.clone(),
                next_percent,
                committed,
            )
        };

        if committed {
            if let Some(props) = route.props_after {
                if let Some(body) = self
                    .state
                    .pending_sends
                    .get_mut(&temporary_id)
                    .and_then(|send| send.body.as_mut())
                {
                    body["props"] = props;
                }
            }
            let crate::pending_send::TimelineReadbackContext {
                window_token,
                causation_id,
            } = timeline_readback;
            if let Some(window_token) = window_token {
                // Explicit retry contexts remain bound to their original window.
                self.refresh_attached_timeline(channel_id, &window_token, causation_id, out)?;
            } else {
                // Normal sends carry no window token. Resolve the actual attachment;
                // "latest" is a literal token, not an alias for the active window.
                self.refresh_attached_latest_timeline_with_deferred_send(
                    channel_id,
                    causation_id,
                    None,
                    out,
                )?;
            }
        }
        if let Some(percent) = next_percent {
            if let Some(state) = self.state.file_upload_progress.get_mut(&route.upload_corr) {
                state.queued_latest = Some(percent);
            }
            self.enqueue_media_progress(&temporary_id, route.upload_corr);
        } else if let Some(pending) = self
            .state
            .media_put_after_progress
            .remove(&route.upload_corr)
        {
            self.finish_file_upload_progress(route.upload_corr);
            self.persist_media_complete_stage(pending, out)?;
        }
        Ok(())
    }

    pub(crate) fn finish_file_upload_progress(&mut self, upload_corr: Correlation) {
        self.state.media_put_after_progress.remove(&upload_corr);
        if let Some(state) = self.state.file_upload_progress.remove(&upload_corr) {
            if let Some((persist_corr, _)) = state.inflight_persist {
                self.state
                    .file_upload_progress_persists
                    .remove(&persist_corr);
            }
        }
    }

    pub(super) fn persist_file_upload_progress(
        &mut self,
        upload_corr: Correlation,
        percent: u8,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let temporary_id = self
            .state
            .file_upload_progress
            .get(&upload_corr)
            .map(|state| state.temporary_id.clone())
            .ok_or_else(|| ImError::Parse("missing file upload progress state".to_string()))?;
        let target = self
            .state
            .file_upload_progress
            .get(&upload_corr)
            .map(|state| state.target.clone())
            .ok_or_else(|| ImError::Parse("missing progress target".to_string()))?;
        let mut props_after = None;
        let op = if matches!(
            target,
            UploadTarget::RichImage { .. } | UploadTarget::RichVideo { .. }
        ) {
            let mut props = self
                .state
                .pending_sends
                .get(&temporary_id)
                .and_then(|send| send.body.as_ref())
                .and_then(|body| body.get("props"))
                .cloned()
                .ok_or_else(|| ImError::Parse("missing rich upload props".to_string()))?;
            target.apply_upload_state(&mut props, "uploading", Some(u64::from(percent)))?;
            // Only accepted 5% steps reach this bounded (at most MAX_RICH_MEDIA) snapshot;
            // raw byte callbacks and target lookup remain O(1).
            let weighted = UploadTarget::message_upload_progress(&props)?;
            props["uploadProgress"] = serde_json::json!(weighted);
            let op = crate::pending_send::props_persist_op(&temporary_id, props.to_string());
            props_after = Some(props);
            op
        } else {
            crate::pending_send::upload_progress_persist_op(&temporary_id, percent)
        };
        let persist_corr = self.alloc_corr_internal();
        self.track_media_write(&temporary_id, persist_corr);
        let state = self
            .state
            .file_upload_progress
            .get_mut(&upload_corr)
            .ok_or_else(|| ImError::Parse("missing file upload progress state".to_string()))?;
        state.inflight_persist = Some((persist_corr, percent));
        self.state.file_upload_progress_persists.insert(
            persist_corr,
            FileUploadProgressPersist {
                props_after,
                upload_corr,
                percent,
            },
        );
        out.push(Effect::Persist {
            corr: persist_corr,
            ops: vec![op],
        });
        Ok(())
    }
}
