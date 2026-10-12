//! HTTP authority uses the same durable member projection fence as private WS updates.
use crate::{
    channel_update::MemberChannelUpdate,
    error::ImError,
    module::ImModule,
    state::{ChannelId, CorrelationContext},
};
use helix_core::{effect::Effect, tick::PortOutcome, EffectSink};
use serde_json::Value;

impl ImModule {
    pub(super) fn reaction_unread_http_reply(
        &mut self,
        req_id: String,
        channel: Option<&str>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let parsed = (|| {
            let channel_id = channel.and_then(ChannelId::from_str)?;
            if self.config.auth_user_id.is_empty()
                || self.config.company_id.is_empty()
                || !self.render_scope.visible(&self.config, channel_id.as_str())
            {
                return None;
            }
            let PortOutcome::Ok(reply) = outcome else {
                return None;
            };
            let raw = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()).ok()?;
            let body: Value = serde_json::from_slice(&raw).ok()?;
            if body["status"] != "SUCCESS" {
                return None;
            }
            let data = &body["data"];
            let count = data["count"].as_i64().filter(|count| *count >= 0)?;
            let target = data.get("nextTarget")?;
            if count == 0 {
                if !target.is_null() {
                    return None;
                }
            } else if target["postId"]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .is_none()
                || !target["revision"]
                    .as_str()
                    .is_some_and(crate::outbound::engagement::reaction_unread::valid_revision)
            {
                return None;
            }
            let member = &data["memberProjection"];
            if member["channelId"].as_str() != Some(channel_id.as_str())
                || member["userId"].as_str() != Some(self.config.auth_user_id.as_str())
            {
                return None;
            }
            let (row, projection) = crate::channel_update::member_channel_from_update_channel(
                member,
                channel_id,
                &self.config.auth_user_id,
                now_ms,
            )?;
            if projection.reaction_unread_count != Some(count) {
                return None;
            }
            let (revision, _) = crate::channel_update::require_member_projection_identity(
                &projection,
                "reaction unread",
            )
            .ok()?;
            Some((channel_id, body, row, projection, revision))
        })();
        let Some((channel_id, body, row, projection, revision)) = parsed else {
            out.push(crate::read_relay::emit_read_error(
                &req_id,
                "REACTION_UNREAD_REQUEST_FAILED",
            ));
            return Ok(());
        };
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ReactionUnreadProjection {
                req_id,
                channel_id,
                body,
                expected: Box::new(projection),
                readback: false,
            },
        );
        out.push(Effect::PersistAtomic {
            corr,
            ops: crate::acl::to_effect::canonical_member_projection_ops(
                channel_id,
                &self.config.auth_user_id,
                revision,
                row,
            ),
        });
        Ok(())
    }

    pub(super) fn reaction_unread_projection_reply(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        body: Value,
        expected: Box<MemberChannelUpdate>,
        readback: bool,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            out.push(crate::read_relay::emit_read_error(
                &req_id,
                "REACTION_UNREAD_PERSIST_FAILED",
            ));
            return Ok(());
        };
        if !self.render_scope.visible(&self.config, channel_id.as_str()) {
            out.push(crate::read_relay::emit_read_error(
                &req_id,
                "RENDER_SCOPE_MISMATCH",
            ));
            return Ok(());
        }
        if !readback {
            let corr = self.alloc_corr_internal();
            self.state.corr_map.insert(
                corr,
                CorrelationContext::ReactionUnreadProjection {
                    req_id,
                    channel_id,
                    body,
                    expected,
                    readback: true,
                },
            );
            out.push(Effect::Persist {
                corr,
                ops: vec![crate::channel_write::message_v3_member_read_op(
                    channel_id,
                    &self.config.auth_user_id,
                )],
            });
            return Ok(());
        }
        let matches = crate::channel_update::member_channel_from_reply(
            reply.0.as_ref(),
            channel_id,
            &self.config.auth_user_id,
            now_ms,
        )
        .is_some_and(|actual| {
            actual.projection_revision == expected.projection_revision
                && actual.effect_id == expected.effect_id
                && crate::channel_update::member_projection_matches(&expected, &actual)
        });
        if !matches {
            out.push(crate::read_relay::emit_read_error(
                &req_id,
                "REACTION_UNREAD_PROJECTION_CHANGED",
            ));
            return Ok(());
        }
        crate::ws::handlers::member_projection::emit_committed_projection(
            reply.0.as_ref(),
            channel_id,
            &expected,
            &self.config.auth_user_id,
            now_ms,
            out,
        )?;
        out.push(crate::read_relay::emit_read_body(&req_id, body));
        Ok(())
    }
}
