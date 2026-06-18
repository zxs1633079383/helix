//! `posts_update` action handler（type=2/3 批量编辑/撤回，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:226 handle_batch_update` → `message_service::handle_batch_update`
//! 保存所有 posts + emit `im:post:batch-updated`；cursor 由顶层 `data.event_seq` 推进
//! （post.rs:266，posts_update 帧 event_seq 在 TOP-LEVEL，不在 per-post props）。
//!
//! wire：`data.posts: [Post]`。per-post `revoke==true` → type=3（revoke_op），否则 type=2
//! （edit_content_op）。cursor 推进一次（顶层 event_seq，复用 frame.event_seq 三级 fallback）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::parser::extract_post_fields;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const POSTS_UPDATE_ACTION: &str = "posts_update";

struct PostsUpdateHandler;

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

        // 逐 post 落库（revoke → type3；否则 type2 内容编辑）。msg_id 缺 → 跳该条。
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
            out.push(helix_core::Effect::PersistFire { ops: vec![op] });
        }

        // channel_id 顶层兜底（posts 可能全无 channelId 字段）。
        let channel_id = channel_id.or_else(|| {
            data.get("channelId")
                .or_else(|| data.get("channel_id"))
                .and_then(serde_json::Value::as_str)
                .and_then(ChannelId::from_str)
        });

        // cursor 推进一次（顶层 event_seq，经 gate 严格 +1）+ emit batch-updated（best-effort）。
        // 内容 patch 已逐条立即落库（上方循环），gate 仅推 cursor、不落 message、不 emit
        // （emit 由本 handler 统一 batch-updated 一次）。
        // A4：事件名 + payload 形态对齐现网 handle_batch_update（im:post:batch-updated + {posts:[...]}）。
        if let Some(cid) = channel_id {
            super::gate::gate_advance_cursor_only(ctx, frame, cid, out)?;
            out.push(crate::acl::to_effect::emit_post_batch_updated(
                cid,
                data.get("posts").unwrap_or(&serde_json::Value::Null),
            ));
        }
        Ok(())
    }
}

static POSTS_UPDATE_HANDLER: PostsUpdateHandler = PostsUpdateHandler;

#[distributed_slice(WS_HANDLERS)]
static POSTS_UPDATE_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POSTS_UPDATE_ACTION,
    handler: &POSTS_UPDATE_HANDLER,
};
