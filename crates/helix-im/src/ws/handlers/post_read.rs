//! `post_read` action handler（type=6 已读位覆盖式写，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:117 handle_read` → `Message::update_fields_by_message_id`
//! 覆盖式写 `read_bits`（服务端权威单调，**非**应用层按位 OR）→ emit `im:post:read`。
//!
//! wire（`PostReadData` types.rs:354）：`postId` / `readMap`(→read_bits) / `createAt` / `updateAt`。
//!
//! BLOCKING-2：落库（`apply_read_op` 覆盖式 read_bits）+ emit `im:post:read` + cursor 推进**全由
//! channel gate** 完成（type6，kind-aware）——cursor 走严格 +1 / drop / buffer+1s-gate，**不再**用
//! `try_advance` MAX-jump（会跨过两条 read 间漏掉的 post 空洞 → 消息丢失，违 spec §S6）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::parser::extract_post_fields;
use crate::state::ChannelId;
use crate::sync_session::EventKind;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const POST_READ_ACTION: &str = "post_read";

struct PostReadHandler;

impl WsMessageHandler for PostReadHandler {
    fn action(&self) -> &'static str {
        POST_READ_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 边界零信任：缺 data → no-op（不可识别 payload 静默忽略，helix-im 不变量 4）。
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };

        // postId 是覆盖式 read_bits 写的 WHERE 键（msg_id）。
        let post_id = data
            .get("postId")
            .or_else(|| data.get("post_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        // channel_id（cursor 推进锚 + emit）：兼容 camel/snake；缺/未注册 → no-op。
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        // type6 fields：read_bits 取 readMap（post_read 专用键）→ readBits → read_bits 兜底。
        // extract_post_fields 不识别 readMap，故先抽 read_bits 覆盖进 fields（边界零信任，缺省空串
        // = 清空已读位语义由服务端定）。msg_id = postId（patch WHERE 键）。
        let mut fields = extract_post_fields(data);
        if fields.read_bits.is_empty() {
            if let Some(rm) = data
                .get("readMap")
                .or_else(|| data.get("read_map"))
                .and_then(serde_json::Value::as_str)
            {
                fields.read_bits = rm.to_string();
            }
        }

        let _ = super::gate::gate_ingest_content_event(
            ctx,
            frame,
            channel_id,
            EventKind::PostRead,
            fields,
            post_id,
            out,
        );
        Ok(())
    }
}

static POST_READ_HANDLER: PostReadHandler = PostReadHandler;

#[distributed_slice(WS_HANDLERS)]
static POST_READ_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POST_READ_ACTION,
    handler: &POST_READ_HANDLER,
};
