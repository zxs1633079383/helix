//! `post_update` action handler（type=2 单条消息编辑，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:171 handle_update` → `message_service::handle_post_update`
//! （event_type=2 / "post_edited"）→ emit `im:post:updated` + 经 `on_ws_event` 推 cursor。
//!
//! wire：`data` 即 post 对象（顶层 `id` / `channelId` / `message` / `props` / `event_seq`）。
//! 落库：`edit_content_op`（**仅覆盖内容列** UPDATE WHERE id，保留本地 read_bits/send_status，
//! 对齐 C2 BLOCKING-1 不擦已读位）。建话题 makeTopic 也走 post_update（spec §S1）。
//!
//! BLOCKING-2：cursor 推进经 **channel gate**（严格 +1 / drop / buffer+1s-gate），与 post handler
//! 的 `ch.ingest` 同口径——**不再**用 `try_advance` MAX-jump（会跨过两条 edit 间漏掉的 post 空洞
//! → 消息丢失）。gate 已 kind-aware（type2 走 `edit_content_op` patch + emit `im:post:updated`），
//! 故落库 + emit + cursor 全由 gate 完成，本 handler 只解析 + 喂 gate。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::parser::extract_post_fields;
use crate::state::ChannelId;
use crate::sync_session::EventKind;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

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
/// `post` = 单个 post JSON 对象（post_update 的 data 顶层）。落库（`edit_content_op` patch，
/// 保留本地 read_bits）+ emit `im:post:updated` + cursor 推进**全由 gate** 完成（kind-aware）：
/// gate 严格 +1 → 缺口 buffer + arm 1s gate，绝不 MAX-jump 跨过两条 edit 间漏掉的 post 空洞。
/// channel_id 缺 / 未注册 → no-op（边界零信任，与 post handler ingest 门控同口径）。
pub(super) fn apply_post_update(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    post: &serde_json::Value,
    out: &mut EffectSink,
) {
    let Some(channel_id) = post
        .get("channelId")
        .or_else(|| post.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return;
    };

    let msg_id = post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // type=2 fields 取自该 post 子树；经 gate ingest（落库 patch + emit + cursor 一并 kind-aware）。
    let fields = extract_post_fields(post);
    let _ = super::gate::gate_ingest_content_event(
        ctx,
        frame,
        channel_id,
        EventKind::PostEdit,
        fields,
        msg_id,
        out,
    );
}

static POST_UPDATE_HANDLER: PostUpdateHandler = PostUpdateHandler;

#[distributed_slice(WS_HANDLERS)]
static POST_UPDATE_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POST_UPDATE_ACTION,
    handler: &POST_UPDATE_HANDLER,
};
