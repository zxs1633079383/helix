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
    /// 校验远端绝对态后持久化，保留原查询范围与恢复预算直到读回终态。
    pub(super) fn reaction_unread_http_reply(
        &mut self,
        req_id: String,
        channel: Option<&str>,
        post_ids: Option<Value>,
        retried: bool,
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
            if let Some(targets) = data.get("targets") {
                let targets = targets.as_array()?;
                let mut unique = std::collections::BTreeSet::new();
                if targets.len() > 100
                    || targets.len() as i64 > count
                    || targets.iter().any(|target| {
                        target["postId"].as_str().is_none_or(|id| {
                            id.trim().is_empty() || id != id.trim() || !unique.insert(id)
                        }) || !target["revision"].as_str().is_some_and(
                            crate::outbound::engagement::reaction_unread::valid_revision,
                        )
                    })
                {
                    return None;
                }
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
                post_ids,
                retried,
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

    /// 旧结果不得覆盖新版；仅真正更高版本可用一次只读重查收敛，其他不一致继续失败。
    pub(super) fn reaction_unread_projection_reply(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        body: Value,
        expected: Box<MemberChannelUpdate>,
        readback: bool,
        post_ids: Option<Value>,
        retried: bool,
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
                    post_ids,
                    retried,
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
        let actual = crate::channel_update::member_channel_from_reply(
            reply.0.as_ref(),
            channel_id,
            &self.config.auth_user_id,
            now_ms,
        );
        let matches = actual.as_ref().is_some_and(|actual| {
            actual.projection_revision == expected.projection_revision
                && actual.effect_id == expected.effect_id
                && crate::channel_update::member_projection_matches(&expected, &actual)
        });
        if !matches {
            let newer = actual.as_ref().is_some_and(|actual| {
                actual.projection_revision > expected.projection_revision
                    && crate::channel_update::require_member_projection_identity(
                        actual,
                        "reaction readback",
                    )
                    .is_ok()
            });
            tracing::warn!(req_id = %req_id, expected_revision = ?expected.projection_revision,
                actual_revision = ?actual.as_ref().and_then(|value| value.projection_revision),
                expected_reaction_count = ?expected.reaction_unread_count,
                actual_reaction_count = ?actual.as_ref().and_then(|value| value.reaction_unread_count),
                same_effect = actual.as_ref().is_some_and(|value| value.effect_id == expected.effect_id),
                newer, retried, "reaction unread durable readback changed");
            if newer && !retried {
                return self.requery_reaction_unread(req_id, channel_id, post_ids, out);
            }
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

    /// 最多追加一次只读 HTTP；原 req_id 与 post_ids 保持，停止时由同一 corr_map 取消。
    fn requery_reaction_unread(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        post_ids: Option<Value>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut args = serde_json::json!({"req_id":req_id, "channel_id":channel_id.as_str()});
        if let Some(ids) = &post_ids {
            args["post_ids"] = ids.clone();
        }
        let corr = self.alloc_corr_internal();
        let effects = crate::commands::handle_outbound(
            "im_reaction_unread",
            &serde_json::to_vec(&args).map_err(|error| ImError::Parse(error.to_string()))?,
            &self.config.api_base_url,
            &self.config.default_api_base_url,
            self.state.connection_id.as_deref(),
            corr,
        )?;
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ReactionUnreadHttp {
                req_id,
                channel_id: Some(channel_id.as_str().to_owned()),
                post_ids,
                retried: true,
            },
        );
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }
}
