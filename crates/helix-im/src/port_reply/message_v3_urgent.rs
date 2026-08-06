//! G-07 expediteMap 的原子提交后读回与终态释放。

use crate::error::ImError;
use crate::module::ImModule;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink};

/// 将连续 G-07 authority 绑定到 expedite_map/cursor 原子提交。
pub(crate) fn queue_commit(
    state: &mut crate::state::ImState,
    corr: Correlation,
    event: crate::sync_session::EventEnvelope,
    out: &mut EffectSink,
) {
    let message_id = event
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(event.fields.id.as_str());
    let urgent_op = crate::channel::write::message_v3_urgent_op(
        message_id,
        event.fields.expedite_map.clone(),
        event.seq.0,
        event.fields.update_at,
    );
    let cursor_op = crate::acl::to_effect::advance_cursor_op(event.channel_id, event.seq);
    state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::MessageV3UrgentPersist {
            event: Box::new(event),
        },
    );
    out.push(Effect::PersistAtomic {
        corr,
        ops: vec![urgent_op, cursor_op],
    });
}

impl ImModule {
    /// 提交 expedite_map/cursor 后推进内存 cursor，并读取 durable message 绝对态。
    pub(super) fn handle_message_v3_urgent_persist_reply(
        &mut self,
        event: crate::sync_session::EventEnvelope,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            if let Some(channel) = self.state.channels.get_mut(&event.channel_id) {
                channel.restore_message_v3_post(event, out);
            }
            return Ok(());
        }
        let message_id = event
            .msg_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(event.fields.id.as_str())
            .to_string();
        let next = self
            .state
            .channels
            .get_mut(&event.channel_id)
            .and_then(|channel| channel.commit_message_v3_post(event.seq));
        self.state
            .invalidate_recent_message_coverage(event.channel_id);
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::MessageV3UrgentReadback {
                message_id: message_id.clone(),
            },
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "message",
                key_col: "id",
                key_val: SqlValue::Text(message_id),
            })],
        });
        if let Some(next_event) = next {
            self.queue_next_message_v3_event(next_event, out)?;
        }
        Ok(())
    }

    /// 从 durable message row 发布 expediteMap 终态与会话级加急信号，并释放旧 action 请求占位。
    ///
    /// 出站序列固定两条（visual 参考包 `post/mv3-g05c` / `post/mv3-g05d` outbound sequence 1 + 2）：
    /// 1. `im:post:updated` —— 绝对 expediteMap，气泡加急标与「已读」动作由它派生。
    /// 2. `im:channel:update` —— 会话级加急标识，会话列表闪电 / 头像红框只读 `urgentPostList`。
    pub(super) fn handle_message_v3_urgent_readback_reply(
        &mut self,
        message_id: String,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            return Ok(());
        };
        let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
            .map_err(|error| ImError::Parse(format!("urgent readback: {error}")))?;
        let Some(row) = rows.first() else {
            return Ok(());
        };
        let Some(event) = crate::event::post::urgent_update_from_row(row)? else {
            return Ok(());
        };
        out.push(event.into_effect());
        if let Some(signal) =
            urgent_channel_signal_from_row(row, self.config.auth_user_id.as_str())?
        {
            out.push(signal.into_effect());
        }
        let _confirmed_message_id = message_id;
        Ok(())
    }
}

/// 从 durable expediteMap 派生当前 viewer 的会话级加急信号（sequence 2）。
///
/// `urgentPostList` 是 member-scoped 绝对投影：只有当前 viewer 在 recipients 中且仍未确认，
/// 才能点亮该账号的会话闪电；发送者和非收件人必须收到空列表。不能把“其他人仍未确认”
/// 当成当前账号的 badge，否则 sender 会错误看到自己发出的加急。
/// `urgentCount` 是死字段（会话列表只读 `urgentPostList`），本函数永不产出它。
fn urgent_channel_signal_from_row(
    row: &helix_core::effect::Row,
    viewer_user_id: &str,
) -> Result<Option<crate::event::MessageV3Event>, ImError> {
    let (Some(post_id), Some(channel_id), Some(expedite_map)) = (
        text_column(row, "id"),
        text_column(row, "channel_id"),
        text_column(row, "expedite_map")
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object),
    ) else {
        return Ok(None);
    };
    let data = if has_pending_recipient(&expedite_map, viewer_user_id) {
        serde_json::json!({
            "channelId": channel_id,
            "userId": viewer_user_id,
            "projectionAuthority": "server-member-absolute",
            "memberScope": "current_user",
            "urgentPostList": [post_id],
            "urgentCurrentName": requester_display_name(&expedite_map),
            "hasUrgentPost": true,
        })
    } else {
        serde_json::json!({
            "channelId": channel_id,
            "userId": viewer_user_id,
            "projectionAuthority": "server-member-absolute",
            "memberScope": "current_user",
            "urgentPostList": [],
            "urgentCurrentName": "",
            "hasUrgentPost": false,
        })
    };
    crate::event::channel::update(data).map(Some)
}

/// 判断当前 viewer 是否仍有未确认的加急目标；缺少 viewer 或 recipient 身份时 fail-closed。
fn has_pending_recipient(expedite_map: &serde_json::Value, viewer_user_id: &str) -> bool {
    if viewer_user_id.is_empty() {
        return false;
    }
    expedite_map
        .get("recipients")
        .and_then(serde_json::Value::as_object)
        .and_then(|recipients| recipients.get(viewer_user_id))
        .is_some_and(|recipient| {
            recipient
                .get("status")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|status| status <= 0)
        })
}

/// 读取加急发起人显示名（会话摘要 `${urgentCurrentName}加急了你` 的唯一来源）。
///
/// 服务端未下发显示名时降级为空串——绝不回落成 userId（UI 会把它当人名直接渲染）。
fn requester_display_name(expedite_map: &serde_json::Value) -> &str {
    expedite_map
        .get("sender")
        .and_then(serde_json::Value::as_object)
        .and_then(|sender| {
            ["name", "nickname", "userName", "displayName"]
                .iter()
                .find_map(|key| sender.get(*key))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

/// 从 driver row 读取文本列，不接受隐式类型转换。
fn text_column<'a>(row: &'a helix_core::effect::Row, column: &str) -> Option<&'a str> {
    row.iter().find_map(|(name, value)| {
        (name == column)
            .then_some(value)
            .and_then(|value| match value {
                helix_core::effect::SqlValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::has_pending_recipient;
    use serde_json::json;

    #[test]
    fn sender_does_not_receive_recipient_urgent_badge() {
        let expedite_map = json!({
            "sender": { "id": "444", "name": "破坏者" },
            "recipients": {
                "447": { "status": 0 },
                "678": { "status": 0 }
            }
        });

        assert!(!has_pending_recipient(&expedite_map, "444"));
        assert!(has_pending_recipient(&expedite_map, "447"));
    }

    #[test]
    fn confirmed_recipient_does_not_keep_urgent_badge() {
        let expedite_map = json!({
            "recipients": {
                "447": { "status": 1 }
            }
        });

        assert!(!has_pending_recipient(&expedite_map, "447"));
        assert!(!has_pending_recipient(&expedite_map, ""));
    }
}
