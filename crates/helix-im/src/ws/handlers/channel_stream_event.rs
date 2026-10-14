//! Canonical durable channel stream event.

use helix_core::{Correlation, Effect, EffectSink};

use crate::error::ImError;
use crate::state::{ChannelId, CorrelationContext, ImState};
use crate::sync_session::{EventEnvelope, EventKind, PostFields};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_STREAM_EVENT_ACTION: &str = "channel_stream_event";

struct ChannelStreamEventHandler;

impl WsMessageHandler for ChannelStreamEventHandler {
    fn action(&self) -> &'static str {
        CHANNEL_STREAM_EVENT_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(event) = parse_stream_event(frame.data_required()?, ctx.auth_user_id)? else {
            return Ok(());
        };
        let needs_hydration = requires_chain_post_hydration(&event);
        if needs_hydration {
            let key = chain_post_event_key(&event);
            if ctx.state.pending_chain_post_event_ids.contains(&key)
                || ctx.state.seen_chain_post_event_ids.contains(&key)
            {
                return Ok(());
            }
        }
        let channel_id = event.channel_id;
        let seq = event.seq;
        // 专用接龙投影可能先推进共享 cursor；完整 message 事实仍须独立补全。
        if needs_hydration
            && ctx
                .state
                .channels
                .get(&channel_id)
                .is_some_and(|channel| channel.cursor.value() >= seq)
        {
            let corr = ctx.alloc_corr();
            return queue_or_hydrate_stream_commit(ctx.state, corr, event, ctx.api_base_url, out);
        }
        let admitted = ctx
            .state
            .channels
            .entry(channel_id)
            .or_insert_with(|| crate::channel::Channel::new(channel_id, 0))
            .admit_message_v3_post(event, out)?;
        if let Some(event) = admitted {
            let corr = ctx.alloc_corr();
            queue_or_hydrate_stream_commit(ctx.state, corr, event, ctx.api_base_url, out)?;
        }
        super::gate::trigger_backfill_if_large_gap(ctx, channel_id, seq, out);
        Ok(())
    }
}

/// 只为没有完整 post 的文字接龙新增/编辑获取消息事实；普通消息、redacted 和终态保持原路径。
pub(crate) fn requires_chain_post_hydration(event: &EventEnvelope) -> bool {
    !event.redacted
        && matches!(event.kind, EventKind::PostUpsert | EventKind::PostEdit)
        && event.fields.id.is_empty()
        && serde_json::from_str::<serde_json::Value>(&event.event_payload)
            .ok()
            .and_then(|payload| {
                payload
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|action| action.starts_with("im:post_chain:"))
}

/// 去重键只关联当前事件，不能充当消息主键。
pub(crate) fn chain_post_event_key(event: &EventEnvelope) -> String {
    if event.event_id.is_empty() {
        format!("{}:{}", event.channel_id.as_str(), event.seq.0)
    } else {
        event.event_id.clone()
    }
}

/// 复用 posts/get 的出站协议，完整消息读取成功前不生成空白 upsert 或 cursor commit。
pub(crate) fn queue_or_hydrate_stream_commit(
    state: &mut ImState,
    corr: Correlation,
    event: EventEnvelope,
    api_base_url: &str,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    if !requires_chain_post_hydration(&event) {
        queue_stream_commit(state, corr, event, out);
        return Ok(());
    }
    let post_id = event
        .msg_id
        .as_deref()
        .filter(|id| crate::state::is_canonical_post_id(id))
        .ok_or_else(|| ImError::Parse("chain stream requires canonical msg_id".to_string()))?;
    let effects = crate::commands::handle_outbound(
        "im_get_posts",
        &serde_json::to_vec(&serde_json::json!({"post_ids":[post_id]}))
            .map_err(|error| ImError::Serialize(error.to_string()))?,
        api_base_url,
        "",
        state.connection_id.as_deref(),
        corr,
    )?;
    state
        .pending_chain_post_event_ids
        .insert(chain_post_event_key(&event));
    state.corr_map.insert(
        corr,
        CorrelationContext::CanonicalChainPostHydration {
            event: Box::new(event),
        },
    );
    for effect in effects {
        out.push(effect);
    }
    Ok(())
}

pub(crate) fn queue_stream_commit(
    state: &mut ImState,
    corr: Correlation,
    event: EventEnvelope,
    out: &mut EffectSink,
) {
    let terminal = matches!(&event.kind, EventKind::ChannelTerminalClosed);
    let mut ops = if terminal {
        vec![crate::acl::to_effect_s1::channel_set_cols_op(
            event.channel_id,
            vec![
                ("is_active", helix_core::effect::SqlValue::Integer(0)),
                (
                    "delete_at",
                    helix_core::effect::SqlValue::Integer(event.occurred_at.max(0)),
                ),
            ],
        )]
    } else if event.redacted || matches!(&event.kind, EventKind::Other(_)) {
        Vec::new()
    } else {
        vec![crate::channel::event_to_storage_op(&event)]
    };
    if terminal {
        ops.push(crate::acl::to_effect::closed_channel_op(event.channel_id));
    }
    ops.push(if terminal {
        crate::acl::to_effect::terminal_tombstone_and_cursor_op(event.channel_id, event.seq)
    } else {
        crate::acl::to_effect::advance_cursor_op(event.channel_id, event.seq)
    });
    state.corr_map.insert(
        corr,
        CorrelationContext::CanonicalStreamPersist {
            event: Box::new(event),
        },
    );
    out.push(Effect::PersistAtomic { corr, ops });
}

pub(crate) fn parse_stream_event(
    data: &serde_json::Value,
    viewer_user_id: &str,
) -> Result<Option<EventEnvelope>, ImError> {
    let Some(channel_id) = data
        .get("channel_id")
        .or_else(|| data.get("channelId"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return Ok(None);
    };
    let Some(stream_seq) = data
        .get("stream_seq")
        .or_else(|| data.get("streamSeq"))
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value > 0 && *value <= i64::MAX as u64)
    else {
        return Ok(None);
    };
    let event_type = data
        .get("event_type")
        .or_else(|| data.get("eventType"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or_default();
    let redacted = data
        .get("redacted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let kind = if redacted {
        EventKind::Other(event_type)
    } else {
        crate::ws::parser::parse_event_kind(event_type)?
    };
    let payload = data.get("payload").filter(|value| !value.is_null());
    let occurred_at = payload
        .and_then(|value| value.get("deleteAt").or_else(|| value.get("delete_at")))
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= 0)
        .unwrap_or_default();
    let post = payload.and_then(|value| value.get("post")).or(payload);
    let fields = if redacted {
        PostFields::default()
    } else {
        post.map(crate::ws::parser::extract_post_fields)
            .unwrap_or_default()
    };
    let msg_id = (!redacted)
        .then(|| {
            data.get("msg_id")
                .or_else(|| data.get("msgId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .flatten();
    let payload_json = if redacted {
        String::new()
    } else {
        payload
            .and_then(|value| serde_json::to_string(value).ok())
            .unwrap_or_default()
    };
    Ok(Some(
        EventEnvelope::new(channel_id, crate::state::Seq(stream_seq), kind, fields)
            .with_msg_id(msg_id)
            .with_event_identity(
                data.get("event_id")
                    .or_else(|| data.get("eventId"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                (!redacted)
                    .then(|| {
                        data.get("actor_id")
                            .or_else(|| data.get("actorId"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .flatten(),
                occurred_at,
                payload_json,
            )
            .with_effect(
                (!redacted)
                    .then(|| {
                        data.get("effect_id")
                            .or_else(|| data.get("effectId"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .flatten(),
                redacted,
            )
            .with_viewer_user_id(viewer_user_id),
    ))
}

static CHANNEL_STREAM_EVENT_HANDLER: ChannelStreamEventHandler = ChannelStreamEventHandler;

#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_STREAM_EVENT_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_STREAM_EVENT_ACTION,
        handler: &CHANNEL_STREAM_EVENT_HANDLER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_event_keeps_only_stream_position() {
        let channel_id = crate::state::test_channel_id(31);
        let event = parse_stream_event(
            &serde_json::json!({
                "channel_id": channel_id.as_str(),
                "stream_seq": 8,
                "event_type": 1,
                "redacted": true,
                "actor_id": "secret",
                "msg_id": "secret-message",
                "payload": {"post": {"id": "secret-message", "message": "secret"}}
            }),
            "viewer",
        )
        .unwrap()
        .unwrap();

        assert_eq!(event.seq.0, 8);
        assert!(matches!(event.kind, EventKind::Other(1)));
        assert!(event.msg_id.is_none());
        assert!(event.actor_id.is_empty());
        assert!(event.fields.id.is_empty());
    }

    #[test]
    fn full_event_uses_explicit_stream_sequence_and_payload() {
        let channel_id = crate::state::test_channel_id(32);
        let event = parse_stream_event(
            &serde_json::json!({
                "channelId": channel_id.as_str(),
                "streamSeq": 9,
                "eventType": 1,
                "eventId": "event-9",
                "actorId": "author",
                "msgId": "message-9",
                "redacted": false,
                "payload": {"post": {"id": "message-9", "channelId": channel_id.as_str(), "message": "hello"}}
            }),
            "viewer",
        )
        .unwrap()
        .unwrap();

        assert_eq!(event.seq.0, 9);
        assert!(matches!(event.kind, EventKind::PostUpsert));
        assert_eq!(event.msg_id.as_deref(), Some("message-9"));
        assert_eq!(event.actor_id, "author");
        assert_eq!(event.fields.message, "hello");
        assert!(event.unread_bump.is_none());
    }
}
