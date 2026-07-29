//! `post_update` action handler（type=2 单条消息编辑，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:171 handle_update` → `message_service::handle_post_update`
//! （event_type=2 / "post_edited"）→ emit `im:post:updated` + 经 `on_ws_event` 推 cursor。
//!
//! wire：`data` 即 post 对象（顶层 `id` / `channelId` / `message` / `props` / `event_seq`）。
//! 落库：普通 edit 走 `edit_content_op`；G-06 quickReply 走独立单列写与 cursor 原子提交，
//! PersistOk 后再读 durable row。两者都保留本地 read_bits/send_status。
//!
//! BLOCKING-2：cursor 推进经 **channel gate**（严格 +1 / drop / buffer+1s-gate），与 post handler
//! 的 `ch.ingest` 同口径——**不再**用 `try_advance` MAX-jump（会跨过两条 edit 间漏掉的 post 空洞
//! → 消息丢失）。gate 已 kind-aware（type2 走 `edit_content_op` patch + emit `im:post:updated`），
//! 故落库 + emit + cursor 全由 gate 完成，本 handler 只解析 + 喂 gate。

use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;
use crate::sync_session::EventKind;
use crate::ws::parser::extract_post_fields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POST_UPDATE_ACTION: &str = "post_update";

struct PostUpdateHandler;

impl WsMessageHandler for PostUpdateHandler {
    fn action(&self) -> &'static str {
        POST_UPDATE_ACTION
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
        apply_post_update(ctx, frame, data, out);
        Ok(())
    }
}

/// 单条 post 编辑——经 channel gate 推进 cursor（type2，BLOCKING-2）。
///
/// `post` = 单个 post JSON 对象。普通 edit 仍由旧 gate 处理；G-06 reaction 与 G-07 urgent
/// 复用 MessageV3 连续性 buffer，严格 +1 后才进入 correlated PersistAtomic 与 durable readback。
/// channel_id 缺 / 未注册 → no-op（边界零信任，与 post handler ingest 门控同口径）。
pub(super) fn apply_post_update(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    data: &serde_json::Value,
    out: &mut EffectSink,
) {
    let post = data.get("post").unwrap_or(data);
    let Some(channel_id) = post
        .get("channelId")
        .or_else(|| post.get("channel_id"))
        .or_else(|| data.get("channelId"))
        .or_else(|| data.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return;
    };
    let msg_id = post
        .get("id")
        .or_else(|| post.get("postId"))
        .or_else(|| post.get("post_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // type=2 fields 取自该 post 子树；经 gate ingest（落库 patch + emit + cursor 一并 kind-aware）。
    let fields = extract_post_fields(post);
    let Some(target_seq) = frame.event_seq() else {
        return;
    };
    if !fields.quick_reply.is_empty() || !fields.expedite_map.is_empty() {
        let event = crate::sync_session::EventEnvelope::new(
            channel_id,
            target_seq,
            EventKind::PostEdit,
            fields,
        )
        .with_msg_id(msg_id)
        .with_viewer_user_id(ctx.auth_user_id);
        let Some(channel) = ctx.state.channels.get_mut(&channel_id) else {
            return;
        };
        let Ok(admitted) = channel.admit_message_v3_post(event, out) else {
            return;
        };
        if let Some(event) = admitted {
            let corr = ctx.alloc_corr();
            if !event.fields.quick_reply.is_empty() {
                crate::port_reply::message_v3_reaction::queue_commit(ctx.state, corr, event, out);
            } else {
                crate::port_reply::message_v3_urgent::queue_commit(ctx.state, corr, event, out);
            }
        }
        super::gate::trigger_backfill_if_large_gap(ctx, channel_id, target_seq, out);
        return;
    }
    let Some(cursor) = ctx
        .state
        .channels
        .get(&channel_id)
        .map(|channel| channel.cursor.value())
    else {
        return;
    };
    let expected = crate::state::Seq(cursor.0.saturating_add(1));
    match target_seq.cmp(&expected) {
        std::cmp::Ordering::Less => return,
        std::cmp::Ordering::Greater => {
            let _ = super::gate::gate_ingest_content_event(
                ctx,
                frame,
                channel_id,
                EventKind::PostEdit,
                fields,
                msg_id,
                out,
            );
            return;
        }
        std::cmp::Ordering::Equal => {}
    }

    let msg_id_ref = msg_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(fields.id.as_str());
    let pending_domain_event = match crate::acl::to_effect::emit_post_updated_for_viewer(
        channel_id,
        target_seq.0,
        msg_id_ref,
        &fields,
        ctx.auth_user_id,
    ) {
        Effect::Emit { event } => event.0.to_vec(),
        _ => return,
    };
    let corr = ctx.alloc_corr();
    let authority_kind = if post
        .get("expediteMap")
        .or_else(|| post.get("expedite_map"))
        .is_some()
    {
        crate::projector::action::AuthorityMutationKind::Urgent
    } else {
        crate::projector::action::AuthorityMutationKind::Reaction
    };
    let pending_action = ctx
        .state
        .pending_authority_actions
        .get(&(authority_kind, msg_id_ref.to_string()))
        .cloned()
        .or_else(|| {
            [
                crate::projector::action::AuthorityMutationKind::Reaction,
                crate::projector::action::AuthorityMutationKind::Urgent,
            ]
            .into_iter()
            .find_map(|kind| {
                ctx.state
                    .pending_authority_actions
                    .get(&(kind, msg_id_ref.to_string()))
                    .cloned()
            })
        });
    ctx.state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::PostUpdateAtomic {
            channel_id,
            target_seq,
            pending_domain_event,
            pending_action,
        },
    );
    out.push(Effect::PersistAtomic {
        corr,
        ops: vec![
            crate::channel::edit_content_op(msg_id_ref, &fields),
            crate::acl::to_effect::advance_cursor_op(channel_id, target_seq),
        ],
    });
}

static POST_UPDATE_HANDLER: PostUpdateHandler = PostUpdateHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_UPDATE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_UPDATE_ACTION,
        handler: &POST_UPDATE_HANDLER,
    }
}
