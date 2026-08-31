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
        let Some(channel_object) = channel.as_object_mut() else {
            tracing::warn!("channel create authority changed shape before member count projection");
            return;
        };
        channel_object.insert(
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

    /// 在 G-15a 持久屏障成功后发布当前 viewer 的 channel 与 roster 绝对态。
    pub(super) fn handle_channel_create_persist_reply(
        &mut self,
        channel_id: ChannelId,
        channel: serde_json::Value,
        member_rows: Vec<serde_json::Value>,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.inflight_channel_creates.remove(&channel_id);
        match outcome {
            PortOutcome::Ok(_) => {
                self.state.committed_channel_creates.insert(channel_id);
                let mut created = build_message_v3_created_projection(
                    channel_id,
                    &channel,
                    &member_rows,
                    self.config.auth_user_id.as_str(),
                );
                if let (Some(request_id), Some(object)) = (causation_id, created.as_object_mut()) {
                    object.insert(
                        "tracing".to_string(),
                        serde_json::json!({ "requestId": request_id }),
                    );
                }
                let members = build_message_v3_members(&member_rows);
                out.push(crate::event::channel::created(created)?.into_effect());
                out.push(
                    crate::event::channel::members(serde_json::json!({
                        "channelId": channel_id.as_str(),
                        "members": members,
                        "leaves": [],
                    }))?
                    .into_effect(),
                );
            }
            PortOutcome::Err(error) => tracing::warn!(
                channel_id = channel_id.as_str(),
                error = ?error,
                "channel create persist failed; suppressing every terminal MessageV3 event"
            ),
        }
        Ok(())
    }
}

/// 构造 Angular 可直接 Upsert 的建群绝对态，并以 owner 开头保持稳定成员顺序。
fn build_message_v3_created_projection(
    channel_id: ChannelId,
    channel: &serde_json::Value,
    member_rows: &[serde_json::Value],
    auth_user_id: &str,
) -> serde_json::Value {
    let owner_id = channel
        .get("owner")
        .and_then(|owner| owner.get("id"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| channel.get("ownerId").and_then(serde_json::Value::as_str))
        .unwrap_or_default();
    let mut member_ids = Vec::with_capacity(member_rows.len());
    let mut seen = std::collections::HashSet::with_capacity(member_rows.len());
    if !owner_id.is_empty() && seen.insert(owner_id) {
        member_ids.push(owner_id);
    }
    for user_id in member_rows
        .iter()
        .filter_map(|member| member.get("user_id").and_then(serde_json::Value::as_str))
    {
        if !user_id.is_empty() && seen.insert(user_id) {
            member_ids.push(user_id);
        }
    }
    let members = build_message_v3_members(member_rows);
    let owner = members
        .iter()
        .find(|member| member.get("userId").and_then(serde_json::Value::as_str) == Some(owner_id))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let viewer_role = members
        .iter()
        .find(|member| {
            member.get("userId").and_then(serde_json::Value::as_str) == Some(auth_user_id)
        })
        .and_then(|member| member.get("role"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("MEMBER");
    let mut projection = serde_json::json!({
        "id": channel_id.as_str(),
        "channelId": channel_id.as_str(),
        "type": channel.get("type").and_then(serde_json::Value::as_str).unwrap_or("O"),
        "displayName": channel.get("displayName").and_then(serde_json::Value::as_str).unwrap_or(channel_id.as_str()),
        "memberIds": member_ids,
        "memberCount": members.len(),
        "members": members,
        "owner": owner,
        "ownerId": owner_id,
        "viewerRole": viewer_role,
        "unreadCount": 0,
        "mentionCount": 0,
    });
    // 权限三件套是 Go 建群默认事实，created 投影必须保留，避免 UI 把群主权限误判为关闭。
    if let Some(projection_object) = projection.as_object_mut() {
        // 建群事件同时保留权限、来源与审计字段，供 Angular 首次渲染直接消费。
        for field in [
            "mentionPermission",
            "noticePermission",
            "topPermission",
            "picture",
            "pictureType",
            "userId",
            "type",
            "source",
            "createAt",
            "createBy",
        ] {
            let Some(value) = channel.get(field) else {
                continue;
            };
            if matches!(
                field,
                "mentionPermission"
                    | "noticePermission"
                    | "topPermission"
                    | "pictureType"
                    | "userId"
                    | "type"
            ) && !value.is_string()
            {
                continue;
            }
            projection_object.insert(field.to_string(), value.clone());
        }
    }
    projection
}

/// 把持久化后的 roster 行转换为 Angular 可直接水合的稳定成员数组。
fn build_message_v3_members(member_rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut members = member_rows
        .iter()
        .filter_map(|member| {
            let user_id = member
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                .filter(|user_id| !user_id.is_empty())?;
            Some(serde_json::json!({
                "userId": user_id,
                "role": member.get("role").and_then(serde_json::Value::as_str).unwrap_or("MEMBER"),
                "nickName": member.get("nick_name").and_then(serde_json::Value::as_str).unwrap_or_default(),
            }))
        })
        .collect::<Vec<_>>();
    if let Some(owner_index) = members
        .iter()
        .position(|member| member.get("role").and_then(serde_json::Value::as_str) == Some("OWNER"))
    {
        members[..=owner_index].rotate_right(1);
    }
    members
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

#[cfg(test)]
mod tests {
    use super::build_message_v3_created_projection;
    use crate::state::ChannelId;
    use serde_json::json;

    /// 建群事件投影必须保留 Go authority 的类型、头像、来源、身份与创建审计字段。
    #[test]
    fn created_projection_preserves_source_and_creator_fields() {
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let channel = json!({
            "id": channel_id.as_str(),
            "type": "P",
            "userId": "444",
            "displayName": "破坏者的快速会议",
            "pictureType": "USER",
            "picture": {"userIds": ["444"]},
            "createAt": 1787120607046_i64,
            "createBy": "444",
            "source": {
                "id": "6a854bde3a8c7230f3223f20",
                "title": "破坏者的快速会议",
                "type": "meeting"
            },
            "owner": {"id": "444"}
        });

        let projection = build_message_v3_created_projection(channel_id, &channel, &[], "444");

        assert_eq!(projection["type"], "P");
        assert_eq!(projection["userId"], "444");
        assert_eq!(projection["pictureType"], "USER");
        assert_eq!(projection["picture"]["userIds"], json!(["444"]));
        assert_eq!(projection["source"]["type"], "meeting");
        assert_eq!(projection["createAt"], 1787120607046_i64);
        assert_eq!(projection["createBy"], "444");
    }
}
