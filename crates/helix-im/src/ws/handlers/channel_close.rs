//! `channel_close` action handler（普通关闭或 viewer-local left 终态，spec §S1）。
//!
//! 行为真源：现网 `handlers/channel.rs:575 handle_close` → `Channel::update_fields(delete_at,
//! is_active=false)` → emit `im:channel:closed{channelId, deleteAt}`。
//!
//! wire（`CloseChannelData` types.rs:374）：普通关闭使用 `channelId` / `deleteAt`；被管理者
//! 移出时复用同一 action，并携带 `terminalReason=left`。普通关闭定点写 `delete_at` +
//! `is_active=0`；viewer-local 离场原子写 `is_remove=1` 并删除当前 viewer 的成员行，
//! 两者均在 PersistOk 后才发布终态。

use helix_core::effect::SqlValue;
use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_CLOSE_ACTION: &str = "channel_close";

struct ChannelCloseHandler;

impl WsMessageHandler for ChannelCloseHandler {
    /// 返回关闭频道 authority 的唯一 WS action。
    fn action(&self) -> &'static str {
        CHANNEL_CLOSE_ACTION
    }

    /// 将可信 close/left tombstone 放入原子持久屏障，提交前不发布终态事件。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        let delete_at = data
            .get("deleteAt")
            .or_else(|| data.get("delete_at"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let is_viewer_left = data
            .get("terminalReason")
            .or_else(|| data.get("terminal_reason"))
            .and_then(serde_json::Value::as_str)
            == Some("left");
        let terminal_seq = data
            .get("eventSeq")
            .or_else(|| data.get("event_seq"))
            .and_then(serde_json::Value::as_u64)
            .filter(|seq| *seq > 0 && *seq <= i64::MAX as u64);
        let terminal_type = data
            .get("eventType")
            .or_else(|| data.get("event_type"))
            .and_then(serde_json::Value::as_u64);
        if !is_viewer_left && terminal_type == Some(7) {
            if let Some(terminal_seq) = terminal_seq {
                let event_id = data
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let event = crate::sync_session::EventEnvelope::new(
                    channel_id,
                    crate::state::Seq(terminal_seq),
                    crate::sync_session::EventKind::ChannelTerminalClosed,
                    crate::sync_session::PostFields::default(),
                )
                .with_event_identity(event_id, None, delete_at.max(0), String::new())
                .with_viewer_user_id(ctx.auth_user_id);
                let admitted = ctx
                    .state
                    .channels
                    .entry(channel_id)
                    .or_insert_with(|| crate::channel::Channel::new(channel_id, 0))
                    .admit_message_v3_post(event, out)?;
                if let Some(event) = admitted {
                    let corr = ctx.alloc_corr();
                    super::channel_stream_event::queue_stream_commit(ctx.state, corr, event, out);
                }
                return Ok(());
            }
        }
        let (transition, cols) = if is_viewer_left {
            (
                crate::state::ChannelLifecycleTransition::Left,
                vec![("is_remove", SqlValue::Integer(1))],
            )
        } else {
            (
                crate::state::ChannelLifecycleTransition::Closed { delete_at },
                vec![
                    ("delete_at", SqlValue::Integer(delete_at)),
                    ("is_closed", SqlValue::Integer(1)),
                    ("is_active", SqlValue::Integer(0)),
                ],
            )
        };

        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelLifecyclePersist {
                channel_id,
                transition,
                causation_id: None,
            },
        );
        let mut ops = vec![crate::acl::to_effect_s1::channel_set_cols_op(
            channel_id, cols,
        )];
        // 被踢 viewer 的终态必须同时清除自己的本地成员投影；普通 close 与无身份回放不产生删行意图。
        if is_viewer_left && !ctx.auth_user_id.is_empty() {
            if let Some(delete_member) = crate::acl::to_effect_s1::delete_channel_members_op(
                channel_id,
                vec![ctx.auth_user_id.to_string()],
            ) {
                ops.push(delete_member);
            }
        }
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
}

static CHANNEL_CLOSE_HANDLER: ChannelCloseHandler = ChannelCloseHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_CLOSE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_CLOSE_ACTION,
        handler: &CHANNEL_CLOSE_HANDLER,
    }
}
