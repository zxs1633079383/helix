//! 发送相关 correlation 注册。
//!
//! `posts/create` HTTP 回包只代表 transport admission，不携带权威 success/post.id；
//! `temporary_id -> post.id` 终态只由 WS post 或权威 read-back 完成。

use helix_core::Correlation;

use crate::state::{CorrelationContext, ImState, TemporaryId};

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
