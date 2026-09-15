//! 发送相关 correlation 注册。
//!
//! `posts/create` HTTP 回包只代表 transport admission，不携带权威 success/post.id；
//! `temporary_id -> post.id` 终态只由 WS post 或权威 read-back 完成。

use helix_core::{Correlation, EffectSink};

use crate::module::ImModule;
use crate::state::{CorrelationContext, ImState, PendingSendReconciliation, TemporaryId};

/// 注册 P1 `OptimisticSend` 路由（乐观落库回报）。
pub(crate) fn register_optimistic_send_corr(
    state: &mut ImState,
    p1_corr: Correlation,
    temporary_id: TemporaryId,
) {
    state
        .corr_map
        .insert(p1_corr, CorrelationContext::OptimisticSend { temporary_id });
}

/// 注册 H1 `OutboundSendHttp` 路由，以区分 admission 成功和 transport 失败。
pub(crate) fn register_outbound_send_http_corr(
    state: &mut ImState,
    h1_corr: Correlation,
    channel_id: crate::state::ChannelId,
    temporary_id: TemporaryId,
    authoritative_readback: bool,
) {
    state.corr_map.insert(
        h1_corr,
        CorrelationContext::OutboundSendHttp {
            channel_id,
            temporary_id,
            authoritative_readback,
        },
    );
}

impl ImModule {
    /// 在 sync 原子写成功后结算已验证的本端 temporaryId，并撤销旧发送计时器。
    pub(crate) fn reconcile_pending_send_after_sync(
        &mut self,
        reconciliation: PendingSendReconciliation,
        out: &mut EffectSink,
    ) -> Result<bool, crate::ImError> {
        let temporary_id = reconciliation.temporary_id;
        let Some(persist_corr) = self
            .state
            .pending_sends
            .get(&temporary_id)
            .map(|pending| pending.persist_corr)
        else {
            return Ok(false);
        };

        let terminal_event = self
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.body.as_ref().map(|body| (pending, body)))
            .map(|(pending, body)| {
                crate::event::post::sent_from_local_body(
                    pending.timeline_readback.causation_id.as_deref(),
                    temporary_id.0.as_str(),
                    reconciliation.server_id.as_str(),
                    self.config.auth_user_id.as_str(),
                    body,
                )
                .map(|event| bytes::Bytes::from(event.into_bytes()))
            })
            .transpose()?;
        let reconcile_corr = self.alloc_corr_internal();
        if let Some(pending) = self.state.pending_sends.get_mut(&temporary_id) {
            pending.reconcile(reconciliation.server_id, reconcile_corr, out);
        }
        self.state.pending_sends.remove(&temporary_id);
        if let Some(persist_corr) = persist_corr {
            self.state.corr_map.remove(&persist_corr);
        }
        self.state.corr_map.insert(
            reconcile_corr,
            match terminal_event {
                Some(terminal_event) => CorrelationContext::AuthoritativeSendTerminalPersist {
                    temporary_id,
                    terminal_event,
                },
                None => CorrelationContext::AuthoritativeSendReconcilePersist { temporary_id },
            },
        );
        Ok(true)
    }
}
