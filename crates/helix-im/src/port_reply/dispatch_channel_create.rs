use crate::error::ImError;
use crate::module::ImModule;
use crate::state::ChannelId;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::EffectSink;

impl ImModule {
    pub(super) fn handle_outbound_channel_create_reply(
        &mut self,
        members: Vec<serde_json::Value>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) {
        let reply = match outcome {
            PortOutcome::Ok(reply) => reply,
            PortOutcome::Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "channel create http failed; waiting for authoritative retry or websocket projection"
                );
                return;
            }
        };
        let mut channel = match decode_created_channel(reply) {
            Ok(channel) => channel,
            Err(error) => {
                tracing::warn!(error = ?error, "channel create http reply cannot build projection");
                return;
            }
        };

        let member_count = members.len() as u64;
        let Some(channel_object) = channel.as_object_mut() else {
            tracing::warn!("channel create http data is not an object");
            return;
        };
        channel_object
            .entry("memberCount".to_string())
            .or_insert_with(|| serde_json::Value::from(member_count));
        channel_object
            .entry("members".to_string())
            .or_insert_with(|| serde_json::Value::Array(members));

        let Some(channel_id) = channel
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            tracing::warn!("channel create http data is missing a valid channel id");
            return;
        };

        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, now_ms, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::channel_member_update::apply_channel_full_and_members(
                &mut ctx, channel_id, &channel, out,
            );
        });

        let Ok(raw) = serde_json::to_vec(&channel) else {
            tracing::warn!("channel create projection serialization failed");
            return;
        };
        out.push(crate::acl::to_effect_s1::emit_channel_created(
            channel_id, &raw,
        ));
        tracing::debug!(
            channel_id = channel_id.as_str(),
            member_count,
            "channel create http reply reconciled into render-ready projection"
        );
    }
}

fn decode_created_channel(reply: &ReplyBytes) -> Result<serde_json::Value, ImError> {
    let raw = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())?;
    let response: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|error| ImError::Parse(format!("channel create response parse error: {error}")))?;

    if let Some(status) = response.get("status").and_then(serde_json::Value::as_str) {
        if !status.eq_ignore_ascii_case("SUCCESS") {
            return Err(ImError::Parse(format!(
                "channel create response status is {status}"
            )));
        }
    }

    if response.get("status").is_some() {
        return response.get("data").cloned().ok_or_else(|| {
            ImError::Parse("channel create response missing object `data`".to_string())
        });
    }
    Ok(response)
}
