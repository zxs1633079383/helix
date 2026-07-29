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
                out.push(Effect::Emit {
                    event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(terminal_event)),
                });
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
}
