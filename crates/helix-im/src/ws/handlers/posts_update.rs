//! `posts_update` action handler（type=2/3 批量编辑/撤回，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:226 handle_batch_update` → `message_service::handle_batch_update`
//! 保存所有 posts + emit `im:post:batch-updated`；cursor 由顶层 `data.event_seq` 推进
//! （post.rs:266，posts_update 帧 event_seq 在 TOP-LEVEL，不在 per-post props）。
//!
//! wire：`data.posts: [Post]`。per-post `revoke==true` → type=3（revoke_op），否则 type=2
//! （edit_content_op）。cursor 推进一次（顶层 event_seq，复用 frame.event_seq 三级 fallback）。

use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;
use crate::ws::parser::extract_post_fields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POSTS_UPDATE_ACTION: &str = "posts_update";

struct PostsUpdateHandler;

#[derive(Default)]
struct RecallEventMeta {
    event_id: String,
    event_seq: u64,
    actor_id: String,
    occurred_at: i64,
}

fn recall_event_meta(
    data: &serde_json::Value,
    post: &serde_json::Value,
    msg_id: &str,
    fallback_seq: u64,
) -> RecallEventMeta {
    let system_event = data
        .get("system_event")
        .or_else(|| data.get("systemEvent"))
        .filter(|event| {
            event
                .get("targetMsgId")
                .and_then(serde_json::Value::as_str)
                .map(|target| target == msg_id)
                .unwrap_or(true)
        })
        .or_else(|| {
            post.get("props")
                .and_then(|props| props.get("system_event"))
        });
    RecallEventMeta {
        event_id: system_event
            .and_then(|event| event.get("eventId"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| data.get("event_id").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string(),
        event_seq: system_event
            .and_then(|event| event.get("eventSeq"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(fallback_seq),
        actor_id: system_event
            .and_then(|event| event.get("actorId"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| data.get("actor_id").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string(),
        occurred_at: system_event
            .and_then(|event| event.get("occurredAt"))
            .and_then(serde_json::Value::as_i64)
            .or_else(|| data.get("occurred_at").and_then(serde_json::Value::as_i64))
            .unwrap_or(0),
    }
}

impl WsMessageHandler for PostsUpdateHandler {
    fn action(&self) -> &'static str {
        POSTS_UPDATE_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };

        // posts 数组缺 / 非数组 → no-op（不可识别 payload 静默忽略）。
        let Some(posts) = data.get("posts").and_then(serde_json::Value::as_array) else {
            return Ok(());
        };

        // metadata 为 additive：新 Go wire 提供 data.system_event；旧帧无该字段时明确 unavailable。
        let fallback_seq = frame.event_seq().map(|seq| seq.0).unwrap_or(0);
        let top_channel_id = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str);

        let Some(target_seq) = frame.event_seq() else {
            return Ok(());
        };
        let channel_for_gate = posts
            .iter()
            .find_map(|post| {
                post.get("channelId")
                    .or_else(|| post.get("channel_id"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(ChannelId::from_str)
            })
            .or(top_channel_id);
        let Some(channel_for_gate) = channel_for_gate else {
            return Ok(());
        };
        let Some(cursor) = ctx
            .state
            .channels
            .get(&channel_for_gate)
            .map(|channel| channel.cursor.value())
        else {
            return Ok(());
        };
        let expected = crate::state::Seq(cursor.0.saturating_add(1));
        match target_seq.cmp(&expected) {
            std::cmp::Ordering::Less => return Ok(()),
            std::cmp::Ordering::Greater => {
                super::gate::gate_advance_cursor_only(ctx, frame, channel_for_gate, out)?;
                return Ok(());
            }
            std::cmp::Ordering::Equal => {}
        }

        // The visible message facts and event cursor form one commit unit. Raw/typed projections are retained
        // as bytes in the corr context and released only after the driver acknowledges the transaction.
        let mut ops = Vec::with_capacity(posts.len().saturating_add(1));
        let mut pending_domain_events = Vec::new();
        let mut pending_action = None;
        let mut channel_id: Option<ChannelId> = None;
        for post in posts {
            let Some(msg_id) = post
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if channel_id.is_none() {
                channel_id = post
                    .get("channelId")
                    .or_else(|| post.get("channel_id"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(ChannelId::from_str);
            }
            let is_revoke = post
                .get("revoke")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let op = if is_revoke {
                crate::channel::revoke_op(msg_id)
            } else {
                crate::channel::edit_content_op(msg_id, &extract_post_fields(post))
            };
            ops.push(op);
            if is_revoke {
                if pending_action.is_none() {
                    pending_action = ctx
                        .state
                        .pending_authority_actions
                        .get(&(
                            crate::projector::action::AuthorityMutationKind::Revoke,
                            msg_id.to_string(),
                        ))
                        .cloned();
                }
                let fields = extract_post_fields(post);
                let meta = recall_event_meta(data, post, msg_id, fallback_seq);
                let projection_channel = post
                    .get("channelId")
                    .or_else(|| post.get("channel_id"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(ChannelId::from_str)
                    .or(top_channel_id);
                if let Some(cid) = projection_channel {
                    if let Effect::Emit { event } =
                        crate::acl::to_effect::emit_post_deleted_for_viewer(
                            cid,
                            meta.event_seq,
                            msg_id,
                            &fields,
                            meta.event_id.as_str(),
                            meta.actor_id.as_str(),
                            meta.occurred_at,
                            ctx.auth_user_id,
                        )
                    {
                        pending_domain_events.push(event.0.to_vec());
                    }
                }
            }
        }

        // channel_id 顶层兜底（posts 可能全无 channelId 字段）。
        let channel_id = channel_id.or_else(|| {
            data.get("channelId")
                .or_else(|| data.get("channel_id"))
                .and_then(serde_json::Value::as_str)
                .and_then(ChannelId::from_str)
        });

        // A4 legacy event stays available for compatibility, but only on the committed side of the barrier.
        if let Some(cid) = channel_id {
            if let Effect::Emit { event } = crate::acl::to_effect::emit_post_batch_updated(
                cid,
                data.get("posts").unwrap_or(&serde_json::Value::Null),
            ) {
                pending_domain_events.push(event.0.to_vec());
            }
        }
        ops.push(crate::acl::to_effect::advance_cursor_op(
            channel_for_gate,
            target_seq,
        ));
        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::PostsUpdateAtomic {
                channel_id: channel_for_gate,
                target_seq,
                pending_domain_events,
                pending_action,
            },
        );
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
}

static POSTS_UPDATE_HANDLER: PostsUpdateHandler = PostsUpdateHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POSTS_UPDATE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POSTS_UPDATE_ACTION,
        handler: &POSTS_UPDATE_HANDLER,
    }
}
