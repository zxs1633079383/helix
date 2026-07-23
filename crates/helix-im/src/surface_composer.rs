//! Post-commit interaction composition for recovery flows.
//!
//! This module receives only committed facts supplied by the sync state machine. It has no
//! storage or transport dependency, so an uncommitted batch cannot fabricate a renderer state.

use crate::interaction_frame::{
    InteractionFrameV2, InteractionRuntimeVm, RuntimeConnectionState, RuntimeRecoveryState,
    RuntimeSyncState, SurfaceMutationV2, UiDirective,
};
use crate::module::ImModule;
use crate::sync_session::RecoveryPhase;
use helix_core::EffectSink;

/// The bounded set of attached slots affected by one committed recovery batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SurfaceChangeSet {
    pub runtime: Option<InteractionRuntimeVm>,
    pub user_mutations: Vec<SurfaceMutationV2>,
    pub channel_bundles: Vec<ChannelInteractionBundle>,
    pub directives: Vec<UiDirective>,
}

/// A channel summary and composer travel together: a renderer cannot enable a send binding from
/// a summary that has not reached the same committed revision boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInteractionBundle {
    pub channel_id: String,
    pub summary: Option<SurfaceMutationV2>,
    pub composer: SurfaceMutationV2,
}

impl SurfaceChangeSet {
    pub fn into_frame(
        self,
        transaction_id: String,
        causation_id: Option<String>,
    ) -> Result<InteractionFrameV2, &'static str> {
        let runtime = self
            .runtime
            .ok_or("committed change set requires runtime")?;
        let mut mutations = self.user_mutations;
        for bundle in self.channel_bundles {
            if bundle.channel_id.is_empty() {
                return Err("channel bundle requires channel id");
            }
            if let Some(summary) = bundle.summary {
                mutations.push(summary);
            }
            mutations.push(bundle.composer);
        }
        crate::projector::build_interaction_frame(
            transaction_id,
            causation_id,
            runtime,
            mutations,
            self.directives,
        )
        .map_err(|_| "invalid composed frame")
    }
}

impl ImModule {
    /// Publish one recovery authority boundary after every scheduled sync and
    /// matching local commit has drained. Subsequent online syncs are ordinary
    /// incremental work and may emit their own targeted dialog patch.
    pub(crate) fn try_finalize_recovery_surface(
        &mut self,
        persisted_corr: Option<helix_core::Correlation>,
        out: &mut EffectSink,
    ) -> Result<bool, crate::error::ImError> {
        let actor_id = self.config.auth_user_id.as_str();
        let session = &self.state.recovery_session;
        if !session.is_collecting_for(actor_id)
            || !self.state.sync_scheduler.is_idle()
            || session.has_pending_commits()
            || !matches!(
                session.phase,
                RecoveryPhase::Recovered | RecoveryPhase::Failed | RecoveryPhase::Blocked
            )
        {
            return Ok(false);
        }

        let recovered = self.state.recovery_session.phase == RecoveryPhase::Recovered;
        self.state.recovery_session.mark_completion_published();
        self.emit_recovery_v2_frame(persisted_corr, out);
        if recovered {
            self.refresh_attached_dialog_list(
                persisted_corr.map(|corr| format!("recovery-complete-{}", corr.raw())),
                out,
            )?;
        }
        Ok(true)
    }

    /// Publish recovery UI only from committed slots.  The caller invokes this
    /// after the matching PersistOk, so a failed or stale persistence reply has
    /// no route to a V2 frame.
    pub(crate) fn emit_recovery_v2_frame(
        &mut self,
        persisted_corr: Option<helix_core::Correlation>,
        out: &mut EffectSink,
    ) {
        let session = &self.state.recovery_session;
        if !session.is_active_for(self.config.auth_user_id.as_str()) {
            return;
        }
        let recovery = match session.phase {
            RecoveryPhase::Recovered => RuntimeRecoveryState::Recovered,
            RecoveryPhase::Failed => RuntimeRecoveryState::Failed,
            RecoveryPhase::Blocked => RuntimeRecoveryState::Blocked,
            RecoveryPhase::Idle => RuntimeRecoveryState::Idle,
            RecoveryPhase::Comparing | RecoveryPhase::Pulling | RecoveryPhase::AwaitingCommit => {
                RuntimeRecoveryState::Syncing
            }
        };
        let Some(user_mutations) = self
            .state
            .dialog_list_frame
            .v2_snapshot_mutations(session.session_epoch)
        else {
            return;
        };
        let runtime = InteractionRuntimeVm {
            connection: RuntimeConnectionState::Online,
            initial_sync: if matches!(recovery, RuntimeRecoveryState::Recovered) {
                RuntimeSyncState::Ready
            } else {
                RuntimeSyncState::Syncing
            },
            recovery,
        };
        let transaction_id = match persisted_corr {
            Some(corr) => format!("recovery:{}:persist:{}", session.session_epoch, corr.raw()),
            None => format!("recovery:{}:compare", session.session_epoch),
        };
        let frame = SurfaceChangeSet {
            runtime: Some(runtime),
            user_mutations,
            ..SurfaceChangeSet::default()
        }
        .into_frame(
            transaction_id,
            persisted_corr.map(|corr| format!("recovery-persist:{}", corr.raw())),
        );
        match frame {
            Ok(frame) => match crate::projector::emit_interaction_frame(frame) {
                Ok(effect) => out.push(effect),
                Err(error) => tracing::warn!(error = %error, "recovery V2 frame suppressed"),
            },
            Err(reason) => tracing::warn!(reason, "recovery V2 frame suppressed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog_frame::DialogListScope;
    use crate::module::ImConfig;
    use crate::state::{ChannelId, Seq};
    use crate::sync_session::{AuthorityHead, CommittedRecoveryHead};
    use helix_core::effect::StorageOp;
    use helix_core::Effect;

    #[test]
    fn recovery_with_one_thousand_channels_publishes_one_bounded_authority_refresh() {
        let mut module = ImModule::new(ImConfig {
            auth_user_id: "actor".to_string(),
            company_id: "team".to_string(),
            ..ImConfig::default()
        });
        module
            .state
            .dialog_list_frame
            .snapshot(DialogListScope::new("actor", "team"), Vec::new());
        module.state.recovery_session.begin("actor");
        assert_eq!(
            module.state.recovery_session.compare(
                CommittedRecoveryHead {
                    cursor: Seq(0),
                    ledger_to_seq: Seq(0),
                    coverage_to_seq: Seq(0),
                },
                AuthorityHead { event_seq: Seq(0) },
            ),
            crate::sync_session::RecoveryComparison::Equal
        );

        for index in 0..1_000 {
            let raw = format!("c{index:025}");
            let channel_id = ChannelId::from_str(raw.as_str()).expect("valid channel id");
            assert!(module.state.sync_scheduler.enqueue(channel_id));
        }

        let mut out = EffectSink::new();
        let mut completion_count = 0;
        while let Some((_channel_id, _trigger)) = module.state.sync_scheduler.next_dispatch() {
            module.state.sync_scheduler.release_window();
            if module
                .try_finalize_recovery_surface(None, &mut out)
                .expect("recovery completion check")
            {
                completion_count += 1;
            }
        }

        assert_eq!(completion_count, 1);
        assert!(!module
            .try_finalize_recovery_surface(None, &mut out)
            .expect("completion is single-shot"));

        let recovery_frames = out
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::Emit { .. }))
            .count();
        let dialog_scans = out
            .as_slice()
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::Persist { ops, .. }
                        if ops.iter().any(
                            |op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel")
                        )
                )
            })
            .count();
        assert_eq!(recovery_frames, 1);
        assert_eq!(dialog_scans, 1);
    }
}
