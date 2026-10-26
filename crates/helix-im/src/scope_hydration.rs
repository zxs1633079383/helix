//! 未知频道的内部 scope hydration 编排。
//!
//! 在线 post 已经落入本地 message 事实后，若频道仍没有可信的 team/owner 行，
//! 这里只负责发起一次既有 `incrementByChannelId` 读链。真实归属只能来自 HTTP
//! 回包后的现有 hydration 写入；本模块不从 post 作者或当前 UI team 推导授权。

use std::collections::HashMap;

use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext, ImState};
use helix_core::effect::{Effect, EffectSink};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingScopeHydration {
    req_id: String,
    auth_user_id: String,
    company_id: String,
}

/// 单频道 scope hydration 的在途去重表；所有状态均为本地确定性数据。
#[derive(Debug, Default)]
pub(crate) struct ScopeHydration {
    pending: HashMap<ChannelId, PendingScopeHydration>,
}

impl ScopeHydration {
    /// 只允许相同身份的同频道请求合并；身份变化时替换旧租约，旧 corr 由 session reset 丢弃。
    pub(crate) fn reserve(
        &mut self,
        channel_id: ChannelId,
        req_id: String,
        auth_user_id: &str,
        company_id: &str,
    ) -> bool {
        if self.pending.get(&channel_id).is_some_and(|pending| {
            pending.auth_user_id == auth_user_id && pending.company_id == company_id
        }) {
            return false;
        }
        self.pending.insert(
            channel_id,
            PendingScopeHydration {
                req_id,
                auth_user_id: auth_user_id.to_string(),
                company_id: company_id.to_string(),
            },
        );
        true
    }

    pub(crate) fn is_active(&self, channel_id: ChannelId, req_id: &str) -> bool {
        self.pending
            .get(&channel_id)
            .is_some_and(|pending| pending.req_id == req_id)
    }

    pub(crate) fn clear(&mut self, channel_id: ChannelId) {
        self.pending.remove(&channel_id);
    }

    pub(crate) fn reset(&mut self) {
        self.pending.clear();
    }
}

impl ImModule {
    /// PersistOk 后为未知频道发起一次内部 hydration；已知外公司频道不重复补水。
    pub(crate) fn start_scope_hydration(&mut self, channel_id: ChannelId, out: &mut EffectSink) {
        if self.config.auth_user_id.is_empty()
            || self
                .render_scope
                .has_trusted_channel_scope(&self.config, channel_id.as_str())
        {
            return;
        }

        let corr = self.alloc_corr_internal();
        let req_id = format!("scope-hydration-{}", corr.raw());
        if !self.scope_hydration.reserve(
            channel_id,
            req_id.clone(),
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        ) {
            return;
        }

        let payload = match serde_json::to_vec(&serde_json::json!({
            "channel_id": channel_id.as_str(),
            "req_id": req_id,
        })) {
            Ok(payload) => payload,
            Err(error) => {
                self.scope_hydration.clear(channel_id);
                tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "scope hydration request payload failed"
                );
                return;
            }
        };

        let effects = match crate::commands::handle_outbound(
            "im_channel_load_increment_by_channel_id",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        ) {
            Ok(effects) => effects,
            Err(error) => {
                self.scope_hydration.clear(channel_id);
                tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "scope hydration request build failed"
                );
                return;
            }
        };

        if !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. }))
        {
            self.scope_hydration.clear(channel_id);
            tracing::warn!(
                channel_id = channel_id.as_str(),
                "scope hydration request produced no HTTP effect"
            );
            return;
        }
        self.state.corr_map.insert(
            corr,
            CorrelationContext::OutboundIncrementHydration {
                req_id,
                emit_channel_increment: true,
                scope_channel: Some(channel_id),
            },
        );
        for effect in effects {
            out.push(effect);
        }
    }
}

/// 既有 increment handler 成功解析后必须留下这条 Persist corr；否则自动补水应释放租约。
pub(crate) fn has_increment_persist(state: &ImState, channel_id: ChannelId, req_id: &str) -> bool {
    state.corr_map.values().any(|context| {
        matches!(
            context,
            CorrelationContext::IncrementHydrationPersist {
                channel_id: pending_channel,
                req_id: pending_req,
                ..
            } if *pending_channel == channel_id && pending_req == req_id
        )
    })
}

/// hydration 的后续 read-back/sync 仍在途时保留同频道去重租约。
pub(crate) fn has_hydration_stage(state: &ImState, channel_id: ChannelId, req_id: &str) -> bool {
    if state
        .hydration_req_ids
        .get(&channel_id)
        .is_some_and(|pending_req| pending_req == req_id)
    {
        return true;
    }
    state.corr_map.values().any(|context| {
        matches!(
            context,
            CorrelationContext::HydrationChannelReadback {
                req_id: pending_req,
                channel_id: pending_channel,
            }
                | CorrelationContext::HydrationMemberReadback {
                    req_id: pending_req,
                    channel_id: pending_channel,
                    ..
                }
                | CorrelationContext::HydrationMessagesReadback {
                    req_id: pending_req,
                    channel_id: pending_channel,
                    ..
                }
                | CorrelationContext::HydrationCursorReadback {
                    req_id: pending_req,
                    channel_id: pending_channel,
                    ..
                } if *pending_channel == channel_id && pending_req == req_id
        )
    })
}

/// 自动内部 req_id 不应以 raw body、null 或业务错误 Emit 伪装成 UI 读结果。
pub(crate) fn drop_auto_read_emits(out: &mut EffectSink, start: usize, req_id: &str) {
    out.retain_mut_from(start, |effect| {
        let Effect::Emit { event } = effect else {
            return true;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(event.0.as_ref()) else {
            return true;
        };
        value["event"] != "im:read:result" || value["data"]["req_id"].as_str() != Some(req_id)
    });
}

#[cfg(test)]
mod tests {
    use super::drop_auto_read_emits;
    use helix_core::effect::{DomainEventBytes, Effect};
    use helix_core::EffectSink;

    #[test]
    fn internal_read_filter_only_removes_matching_request() {
        let mut sink = EffectSink::new();
        for req_id in ["external-1", "scope-hydration-1"] {
            sink.push(Effect::Emit {
                event: DomainEventBytes(bytes::Bytes::from(
                    serde_json::to_vec(&serde_json::json!({
                        "event": "im:read:result",
                        "data": { "req_id": req_id, "body": null }
                    }))
                    .expect("test event must serialize"),
                )),
            });
        }

        drop_auto_read_emits(&mut sink, 0, "scope-hydration-1");

        assert_eq!(sink.as_slice().len(), 1);
        let Effect::Emit { event } = &sink.as_slice()[0] else {
            panic!("read relay event should remain");
        };
        assert!(event
            .0
            .as_ref()
            .windows(b"external-1".len())
            .any(|window| { window == b"external-1" }));
    }
}
