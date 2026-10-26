use crate::module::ImModule;
use helix_core::effect::Effect;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;

impl ImModule {
    /// G-14 在持久化成功后发布当前 viewer 的单条已读终态。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_message_v3_post_read_persist_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        message_id: String,
        receipt_revision: i64,
        read_bits: String,
        terminal_event: Vec<u8>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(_) => {
                self.state
                    .committed_post_reads
                    .insert(message_id.clone(), (receipt_revision, read_bits));
                if let Some(terminal_event) = self.render_ready_post_read_terminal(terminal_event) {
                    out.push(Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(
                            terminal_event,
                        )),
                    });
                }
                self.state.invalidate_recent_message_coverage(channel_id);
            }
            PortOutcome::Err(error) => tracing::warn!(
                channel_id = channel_id.as_str(),
                message_id,
                error = ?error,
                "post_read persist failed; MessageV3 receipt remains unchanged"
            ),
        }
    }

    /// 把 G-14 单视角终态交给 render-ready 回执归一；返回 `None` 表示本 viewer 不应收到该事件。
    ///
    /// 非 JSON 或无 `data` 的终态原样透传：归一层只做投影收敛，不承担协议解码的 fail-closed。
    ///
    /// Go authority 已携带 author、snapshot 与 member 顺序；本接缝只做 O(n) 投影并移除 bitmap。
    fn render_ready_post_read_terminal(&self, terminal_event: Vec<u8>) -> Option<Vec<u8>> {
        let Ok(mut payload) = serde_json::from_slice::<serde_json::Value>(&terminal_event) else {
            return Some(terminal_event);
        };
        let Some(data) = payload.get_mut("data") else {
            return Some(terminal_event);
        };
        if !crate::query::render_ready::receipts::render_ready_post_read(
            data,
            self.config.auth_user_id.as_str(),
        ) {
            return None;
        }
        Some(serde_json::to_vec(&payload).unwrap_or(terminal_event))
    }
}
