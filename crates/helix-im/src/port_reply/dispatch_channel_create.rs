use crate::error::ImError;
use crate::module::ImModule;
use crate::state::ChannelId;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::EffectSink;

impl ImModule {
    /// 将 HTTP create authority 交给与 WS 共用的 G-15a 持久化入口。
    fn queue_channel_create_persist(
        &mut self,
        channel_id: ChannelId,
        channel: serde_json::Value,
        causation_id: Option<String>,
        now_ms: u64,
        out: &mut EffectSink,
    ) {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, now_ms, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::channel_member_update::queue_channel_create_persist(
                &mut ctx,
                channel_id,
                channel,
                causation_id,
                out,
            );
        });
    }

    /// 解析 channel/create HTTP 回包并只排队权威持久化，不提前发布 UI。
    pub(super) fn handle_outbound_channel_create_reply(
        &mut self,
        members: Vec<serde_json::Value>,
        request_id: Option<String>,
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

        {
            let Some(channel_object) = channel.as_object_mut() else {
                tracing::warn!("channel create http data is not an object");
                return;
            };
            channel_object
                .entry("members".to_string())
                .or_insert_with(|| serde_json::Value::Array(members));
        }
        // create 请求只携带被邀请者，HTTP 回包另带 owner。标量必须从
        // 去重后的权威 roster 得出，不能把 user_ids.len() 冒充总成员数。
        let member_count = crate::channel_write::collect_members(&channel).len() as u64;
        channel
            .as_object_mut()
            .expect("channel object was checked above")
            .insert(
                "memberCount".to_string(),
                serde_json::Value::from(member_count),
            );

        let Some(channel_id) = channel
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            tracing::warn!("channel create http data is missing a valid channel id");
            return;
        };

        self.queue_channel_create_persist(channel_id, channel, request_id, now_ms, out);
        tracing::debug!(
            channel_id = channel_id.as_str(),
            member_count,
            "channel create http reply queued behind the durable authority barrier"
        );
    }

    /// 在 G-15a 持久屏障成功后原子发布 dialog 与 summary/roster typed Frame。
    pub(super) fn handle_channel_create_persist_reply(
        &mut self,
        channel_id: ChannelId,
        channel: serde_json::Value,
        member_rows: Vec<serde_json::Value>,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.inflight_channel_creates.remove(&channel_id);
        match outcome {
            PortOutcome::Ok(_) => {
                self.state.committed_channel_creates.insert(channel_id);
                if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
                    if let Some(frame) = self
                        .state
                        .dialog_list_frame
                        .patch_insert_channel(scope, &channel)
                    {
                        out.push(frame);
                    }
                }
                self.project_created_channel_frame(
                    channel_id,
                    &channel,
                    &member_rows,
                    causation_id,
                    now_ms,
                    out,
                )?;
            }
            PortOutcome::Err(error) => tracing::warn!(
                channel_id = channel_id.as_str(),
                error = ?error,
                "channel create persist failed; suppressing every terminal Frame"
            ),
        }
        Ok(())
    }
}

/// 解包并校验 channel/create 的 SUCCESS 权威对象。
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
