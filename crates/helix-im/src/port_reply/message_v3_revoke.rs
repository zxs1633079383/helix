//! G-04 update_channel 持久屏障、复合键读回与 channel 事件释放。

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    /// 撤回 lastPost 原子提交成功后读取当前 viewer 的 channel_member 绝对态。
    pub(super) fn handle_message_v3_revoke_channel_persist_reply(
        &mut self,
        channel_id: ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            return;
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![crate::channel_write::message_v3_member_read_op(
                channel_id,
                self.config.auth_user_id.as_str(),
            )],
        });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::MessageV3RevokeChannelReadback);
    }

    /// 撤回 channel_member 读回成功后只释放 im:channel:update。
    pub(super) fn handle_message_v3_revoke_channel_readback_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(channel) = super::message_v3_post::channel_update_from_member_readback(outcome)?
        {
            out.push(channel.into_effect());
        }
        Ok(())
    }
}
