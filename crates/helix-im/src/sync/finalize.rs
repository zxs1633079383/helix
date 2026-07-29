//! 同步批次完成条件与 MessageV3 终态发布。

use helix_core::EffectSink;

use crate::module::ImModule;
use crate::sync_session::RecoveryPhase;

impl ImModule {
    /// 所有 pong gap 拉取与持久回执排空后发布一次修复终态。
    pub(crate) fn try_finalize_pong_gap(
        &mut self,
        persisted_corr: Option<helix_core::Correlation>,
        out: &mut EffectSink,
    ) -> Result<bool, crate::error::ImError> {
        let Some(committed_change) = self
            .state
            .pong_gap_batch
            .take_completion(self.state.sync_scheduler.is_idle())
        else {
            return Ok(false);
        };
        if committed_change {
            out.push(
                crate::event::sync::gap_repaired(serde_json::json!({
                    "state": "repaired",
                    "source": "pong",
                    "persistedCorrelation": persisted_corr.map(helix_core::Correlation::raw),
                }))?
                .into_effect(),
            );
            self.refresh_channel_list(
                persisted_corr.map(|corr| format!("pong-gap-complete-{}", corr.raw())),
                out,
            )?;
        }
        Ok(true)
    }

    /// 所有恢复拉取与本地提交排空后发布一次恢复终态。
    pub(crate) fn try_finalize_recovery(
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
        out.push(
            crate::event::sync::recovered(serde_json::json!({
                "state": if recovered { "recovered" } else { "failed" },
                "persistedCorrelation": persisted_corr.map(helix_core::Correlation::raw),
            }))?
            .into_effect(),
        );
        if recovered {
            self.refresh_channel_list(
                persisted_corr.map(|corr| format!("recovery-complete-{}", corr.raw())),
                out,
            )?;
        }
        Ok(true)
    }
}
