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
        let crate::send::upload_props::PendingMediaOp::Put(pending) = operation else {
            return false;
        };
        let must_wait = self
            .state
            .file_upload_progress
            .get(&upload_corr)
            .is_some_and(|state| state.inflight_persist.is_some());
        if must_wait {
            self.state
                .media_put_after_progress
                .insert(upload_corr, pending.clone());
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
        if !matches!(target, UploadTarget::File) {
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
                temporary_id,
                channel_id,
                timeline_readback,
                highest_observed_percent: 0,
                committed_percent: 0,
                inflight_persist: None,
                queued_latest: None,
            },
        );
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
        let percent = (raw_percent / 5) * 5;
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

        self.persist_file_upload_progress(upload_corr, percent, out)
    }

    pub(crate) fn handle_file_upload_progress_persist_reply(
        &mut self,
        persist_corr: Correlation,
        route: FileUploadProgressPersist,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let (channel_id, timeline_readback, next_percent, committed) = {
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
                state.channel_id,
                state.timeline_readback.clone(),
                next_percent,
                committed,
            )
        };

        if committed {
            let crate::pending_send::TimelineReadbackContext {
                window_token,
                causation_id,
            } = timeline_readback;
            self.refresh_attached_timeline(
                channel_id,
                window_token.as_deref().unwrap_or("latest"),
                causation_id,
                out,
            )?;
        }
        if let Some(percent) = next_percent {
            self.persist_file_upload_progress(route.upload_corr, percent, out)?;
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

    fn persist_file_upload_progress(
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
        let persist_corr = self.alloc_corr_internal();
        let state = self
            .state
            .file_upload_progress
            .get_mut(&upload_corr)
            .ok_or_else(|| ImError::Parse("missing file upload progress state".to_string()))?;
        state.inflight_persist = Some((persist_corr, percent));
        self.state.file_upload_progress_persists.insert(
            persist_corr,
            FileUploadProgressPersist {
                upload_corr,
                percent,
            },
        );
        out.push(Effect::Persist {
            corr: persist_corr,
            ops: vec![crate::pending_send::upload_progress_persist_op(
                &temporary_id,
                percent,
            )],
        });
        Ok(())
    }
}
