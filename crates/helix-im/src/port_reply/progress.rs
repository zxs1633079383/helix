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

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::effect::StorageOp;
    use helix_core::tick::{PortOutcome, ReplyBytes};
    use helix_core::{Module, Tick, TimerId};

    fn channel_id() -> ChannelId {
        ChannelId::from_str("chfixx00000000000000000001").expect("valid channel")
    }

    #[test]
    // 回归锚: G03a（retry 的非 latest window 在文件进度期间不得退回 latest）
    fn progress_is_latest_wins_and_preserves_retry_window_and_causation() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = channel_id();
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "retry-progress-window".to_string(),
        };
        module.state.timeline_projector.register_attachment(&scope);
        let upload_corr = Correlation::from_raw(700);
        let temporary_id = TemporaryId("tmp-progress".to_string());
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            TimerId::from_raw(700),
            None,
        );
        pending.timeline_readback = crate::pending_send::TimelineReadbackContext {
            window_token: Some("retry-progress-window".to_string()),
            causation_id: Some("retry-progress-request".to_string()),
        };
        module
            .state
            .pending_sends
            .insert(temporary_id.clone(), pending);
        module.register_file_upload_progress(
            upload_corr,
            temporary_id.clone(),
            channel_id,
            &UploadTarget::File,
        );

        let mut out = EffectSink::new();
        module
            .handle(
                &Tick::PortProgress {
                    corr: upload_corr,
                    progress: FileUploadProgress {
                        completed_bytes: 6,
                        total_bytes: 100,
                    },
                },
                1,
                &mut out,
            )
            .unwrap();
        let first_persist = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(|op| {
                        matches!(
                            op,
                            StorageOp::BatchUpdate(spec)
                                if spec.patch.iter().any(|(column, value)|
                                    column == "upload_progress_percent"
                                        && matches!(
                                            value,
                                            helix_core::effect::SqlValue::Integer(5)
                                        ))
                        )
                    }) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .expect("first 5% persist");

        for completed_bytes in [12, 37] {
            out.clear();
            module
                .handle(
                    &Tick::PortProgress {
                        corr: upload_corr,
                        progress: FileUploadProgress {
                            completed_bytes,
                            total_bytes: 100,
                        },
                    },
                    2,
                    &mut out,
                )
                .unwrap();
            assert!(out.is_empty(), "one progress persist stays in flight");
        }

        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: first_persist,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                3,
                &mut out,
            )
            .unwrap();
        let queued_latest = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(|op| {
                        matches!(
                            op,
                            StorageOp::BatchUpdate(spec)
                                if spec.patch.iter().any(|(column, value)|
                                    column == "upload_progress_percent"
                                        && matches!(
                                            value,
                                            helix_core::effect::SqlValue::Integer(35)
                                        ))
                        )
                    }) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .expect("latest queued bucket wins");

        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: queued_latest,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                4,
                &mut out,
            )
            .unwrap();
        let scan_corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"),
                    ) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .expect("PersistOk refreshes attached timeline");

        let rows = serde_json::json!([{
            "temporary_id": temporary_id.0,
            "channel_id": channel_id.as_str(),
            "user_id": "viewer",
            "type": "FILE",
            "message": "",
            "props": "{\"file\":{\"upload\":{\"status\":\"uploading\"}}}",
            "send_status": "sending",
            "upload_progress_percent": 35,
            "create_at": 1
        }]);
        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: scan_corr,
                    outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                        serde_json::to_vec(&rows).unwrap(),
                    ))),
                },
                5,
                &mut out,
            )
            .unwrap();
        let frame = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => {
                    serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                        .ok()
                        .filter(|value| value["event"] == "im:interaction:frame")
                }
                _ => None,
            })
            .expect("readback emits a typed Frame");
        let decoded = crate::interaction_frame::InteractionFrame::decode(
            &serde_json::to_vec(&frame["data"]).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.mutations[0].scope, scope);
        assert_eq!(
            decoded.mutations[0].causation_id.as_deref(),
            Some("retry-progress-request")
        );
        let delivery = &frame["data"]["mutations"][0]["mutation"]["view"]["items"][0]["delivery"];
        assert_eq!(delivery["kind"], "uploading");
        assert_eq!(delivery["progressPercent"], 35);
    }

    #[test]
    fn file_requests_enable_five_percent_progress_but_rich_images_disable_it() {
        let input = crate::send::upload_props::MediaInput {
            local_path: "/tmp/a.bin".to_string(),
            file_name: "a.bin".to_string(),
            content_type: "application/octet-stream".to_string(),
            size: 100,
            sha256: "a".repeat(64),
        };
        let prepared = crate::send::upload_props::PreparedMedia {
            upload_id: "upload-1".to_string(),
            file_id: "file-1".to_string(),
            method: "PUT".to_string(),
            url: "https://upload.invalid/signed".to_string(),
            public_url: "https://cdn.invalid/file-1".to_string(),
            object_key: "file-1".to_string(),
            headers: Vec::new(),
        };

        let file =
            crate::send::upload_props::media_upload_request(&input, &prepared, &UploadTarget::File)
                .unwrap();
        assert_eq!(
            file.progress_policy(),
            helix_core::FileUploadProgressPolicy::PercentStep(5)
        );

        let image = crate::send::upload_props::media_upload_request(
            &input,
            &prepared,
            &UploadTarget::RichImage { index: 0 },
        )
        .unwrap();
        assert_eq!(
            image.progress_policy(),
            helix_core::FileUploadProgressPolicy::Disabled
        );
    }

    #[test]
    fn put_success_waits_for_inflight_progress_persist_before_terminal_stage() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let upload_corr = Correlation::from_raw(702);
        let temporary_id = TemporaryId("tmp-progress-terminal".to_string());
        let channel_id = channel_id();
        module.register_file_upload_progress(
            upload_corr,
            temporary_id.clone(),
            channel_id,
            &UploadTarget::File,
        );

        let mut out = EffectSink::new();
        module
            .handle_file_upload_progress(
                upload_corr,
                FileUploadProgress {
                    completed_bytes: 55,
                    total_bytes: 100,
                },
                &mut out,
            )
            .unwrap();
        let progress_corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("55% progress persist");
        let route = module
            .state
            .file_upload_progress_persists
            .remove(&progress_corr)
            .expect("progress reply route");
        let pending = crate::send::upload_props::PendingMediaPut {
            temporary_id,
            channel_id,
            target: UploadTarget::File,
            input: crate::send::upload_props::MediaInput {
                local_path: "/tmp/report.pdf".to_string(),
                file_name: "report.pdf".to_string(),
                content_type: "application/pdf".to_string(),
                size: 100,
                sha256: "a".repeat(64),
            },
            prepared: crate::send::upload_props::PreparedMedia {
                upload_id: "b".repeat(64),
                file_id: "mf_0123456789abcdefghijklm".to_string(),
                method: "PUT".to_string(),
                url: "https://oss.invalid/signed".to_string(),
                public_url: "https://java.invalid/oss/media/object/mf_0123456789abcdefghijklm"
                    .to_string(),
                object_key: "media/mf_0123456789abcdefghijklm".to_string(),
                headers: Vec::new(),
            },
        };
        let operation = crate::send::upload_props::PendingMediaOp::Put(pending);
        assert!(
            module.defer_media_put_until_progress_commits(upload_corr, &operation),
            "PUT success must wait while an older message progress write is inflight"
        );

        out.clear();
        module
            .handle_file_upload_progress_persist_reply(
                progress_corr,
                route,
                &PortOutcome::Ok(ReplyBytes::default()),
                &mut out,
            )
            .unwrap();

        assert!(out.as_slice().iter().any(|effect| {
            let Effect::Persist { ops, .. } = effect else {
                return false;
            };
            ops.iter().any(|op| {
                matches!(
                    op,
                    StorageOp::BatchUpsert(spec)
                        if spec.table == "pending_media"
                            && spec.rows.iter().flatten().any(|(column, value)|
                                column == "stage"
                                    && matches!(
                                        value,
                                        helix_core::effect::SqlValue::Text(stage)
                                            if stage == "complete"
                                    )
                            )
                )
            })
        }));
        assert!(!module.state.file_upload_progress.contains_key(&upload_corr));
        assert!(!module
            .state
            .media_put_after_progress
            .contains_key(&upload_corr));
    }

    #[test]
    // 回归锚: G03a（retry 再次失败的终态必须返回授权 window）
    fn upload_failure_preserves_retry_window_and_causation_and_clears_stale_progress() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "viewer".to_string();
        let mut module = ImModule::new(config);
        let channel_id = channel_id();
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "retry-failure-window".to_string(),
        };
        module.state.timeline_projector.register_attachment(&scope);

        let temporary_id = TemporaryId("tmp-upload-failed".to_string());
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            TimerId::from_raw(701),
            None,
        );
        pending.status = crate::state::SendStatus::Sending;
        pending.timeline_readback = crate::pending_send::TimelineReadbackContext {
            window_token: Some("retry-failure-window".to_string()),
            causation_id: Some("retry-failure-request".to_string()),
        };
        pending.body = Some(serde_json::json!({
            "temporaryId": temporary_id.0,
            "channelId": channel_id.as_str(),
            "type": "FILE",
            "message": "",
            "props": {
                "file": {
                    "name": "report.pdf",
                    "upload": {"status": "uploading"}
                }
            }
        }));
        module
            .state
            .pending_sends
            .insert(temporary_id.clone(), pending);

        let mut out = EffectSink::new();
        module
            .mark_upload_failed(
                crate::send::upload_props::PendingUpload {
                    temporary_id: temporary_id.clone(),
                    target: UploadTarget::File,
                },
                "oss put failed",
                &mut out,
            )
            .unwrap();

        assert_eq!(
            module
                .state
                .pending_sends
                .get(&temporary_id)
                .map(|pending| pending.status),
            Some(crate::state::SendStatus::UnSend),
            "media failure must leave the in-memory aggregate in a retryable terminal state"
        );
        let failure_persist = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops } => {
                    let row = ops.iter().find_map(|op| match op {
                        StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                            spec.rows.first()
                        }
                        _ => None,
                    })?;
                    let has_unsend = row.iter().any(|(column, value)| {
                        column == "send_status"
                            && matches!(
                                value,
                                helix_core::effect::SqlValue::Text(status)
                                    if status == "unsend"
                            )
                    });
                    let reset_progress = row.iter().any(|(column, value)| {
                        column == "upload_progress_percent"
                            && matches!(value, helix_core::effect::SqlValue::Integer(0))
                    });
                    (has_unsend && reset_progress).then_some(*corr)
                }
                _ => None,
            })
            .expect("failure persist atomically stores failed status and clears stale progress");
        assert!(out.as_slice().iter().any(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                .ok()
                .is_some_and(|value| value["event"] == "im:post:send-failed"),
            _ => false,
        }));

        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: failure_persist,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                2,
                &mut out,
            )
            .unwrap();
        let scan_corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"),
                    ) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .expect("failed status PersistOk refreshes the attached timeline");

        let rows = serde_json::json!([{
            "temporary_id": temporary_id.0,
            "channel_id": channel_id.as_str(),
            "user_id": "viewer",
            "type": "FILE",
            "message": "",
            "props": "{\"file\":{\"name\":\"report.pdf\",\"upload\":{\"status\":\"failed\"}}}",
            "send_status": "unsend",
            "upload_progress_percent": 0,
            "create_at": 1
        }]);
        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: scan_corr,
                    outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                        serde_json::to_vec(&rows).unwrap(),
                    ))),
                },
                3,
                &mut out,
            )
            .unwrap();
        let frame = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => {
                    serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                        .ok()
                        .filter(|value| value["event"] == "im:interaction:frame")
                }
                _ => None,
            })
            .expect("failed readback emits a typed Frame");
        let decoded = crate::interaction_frame::InteractionFrame::decode(
            &serde_json::to_vec(&frame["data"]).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.mutations[0].scope, scope);
        assert_eq!(
            decoded.mutations[0].causation_id.as_deref(),
            Some("retry-failure-request")
        );
        let view = &frame["data"]["mutations"][0]["mutation"]["view"];
        let delivery = &view["items"][0]["delivery"];
        assert_eq!(delivery["kind"], "failed");
        assert_eq!(delivery["retryable"], true);
        assert!(view["items"][0]["actions"]
            .as_array()
            .is_some_and(|actions| actions
                .iter()
                .any(|action| { action["kind"] == "retry" && action["enabled"] == true })));
    }
}
