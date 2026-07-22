//! `post_read` action handler（type=6 已读位覆盖式写，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:117 handle_read` → `Message::update_fields_by_message_id`
//! 覆盖式写 `read_bits`（服务端权威单调，**非**应用层按位 OR）→ emit `im:post:read`。
//!
//! wire（`PostReadData` types.rs:354）：`postId` / `readMap`(→read_bits) / `createAt` / `updateAt`。
//!
//! ## 为什么 post_read 绕开 channel gate（不走 event_seq 严格 +1 排序）
//!
//! `read_bits`（`readMap`）是服务端权威**全量已读位图**——幂等覆盖、order-independent、与消息
//! 序列**解耦**：
//!   - 后端 `post_read` 广播帧**不携带 `event_seq`**（read 不产生 sequenced channel_event；
//!     实证 cses-im-server post_read `data` 仅 `{channelId, postId, readMap, createAt, updateAt}`）；
//!   - 走 `gate_ingest_content_event` 会因 `frame.event_seq() == None` 直接 no-op
//!     → 永不 emit / 永不落库（issue #14 L2 read-receipt ②④ 永红根因）；
//!   - read 是对**既存** message 行 `read_bits` 列的覆盖写（O(1) `UPDATE … WHERE id`），不需要
//!     `+1` cursor 排序、不参与 `flush_contiguous`、**不**推进 cursor——cursor 是消息序列水位，
//!     把无序的旁路已读回执塞进 gate 推进它会污染同步基线（违 spec §S6 cursor 单调语义）。
//!
//! ∴ post_read 走**专用直路**：`apply_read_op`（覆盖 read_bits）+ `emit_post_read`（fat），
//! **不经 gate**。守 HX-C005（数据在手不回读：read_bits 取帧内 readMap，不回查 DB）。
//!
//! content event（`post` / `post_update` / `post_revoke`）仍走 gate 严格 +1（那是带序的消息事件，
//! 序列化对消息正确）——本改**只**让幂等覆盖类 read 旁路，gate 对消息事件的 event_seq 校验不动。

use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;
use crate::ws::parser::extract_post_fields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

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

        // postId：覆盖式 read_bits 写的 WHERE 键（msg_id）+ emit 锚（前端按 msg_id 定位行）。
        // 缺/空 → no-op（无写键 = UPDATE WHERE id="" 命中 0 行无意义，且 emit 无法锚定行）。
        let Some(post_id) = data
            .get("postId")
            .or_else(|| data.get("post_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return Ok(());
        };

        // channelId：emit 投影锚（前端按 channelId 定位频道行）。兼容 camel/snake；缺 → no-op。
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };
        ctx.state.invalidate_recent_message_coverage(channel_id);

        // type6 fields：read_bits 取 readMap（post_read 专用键）→ readBits → read_bits 兜底。
        // extract_post_fields 不识别 readMap，故先抽 read_bits 覆盖进 fields（边界零信任，缺省空串
        // = 清空已读位语义由服务端定）。
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
        let receipt_revision = data
            .get("updateAt")
            .or_else(|| data.get("update_at"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();

        // 专用直路（不经 gate）：
        //   ① 覆盖式落 read_bits（O(1) `UPDATE message SET read_bits WHERE id=postId`；既存行命中，
        //      消息体尚未落地时 0 行 = 安全 no-op，待后续 type1 sync 带其自身 read_bits）；
        //   ② emit `im:post:read`（fat·event_seq=0：read 不参与消息序列，投影 event_seq 仅占位键）。
        // WS 路径用 PersistFire（幂等）。cursor 不推进（read 是旁路回执，非序列事件）。
        out.push(Effect::PersistFire {
            ops: vec![crate::channel::apply_read_op(
                &post_id,
                fields.read_bits.as_str(),
            )],
        });
        out.push(
            crate::acl::to_effect::emit_post_read_with_receipt_revision_for_viewer(
                channel_id,
                0,
                &post_id,
                &fields,
                receipt_revision,
                ctx.auth_user_id,
            ),
        );
        // read_bits 已先以权威覆盖写进入本地存储；随后仍需经过 attached timeline 的
        // local-first readback 重新生成 typed InteractionFrame，不能让 PC 从 raw
        // `im:post:read` 自行拼 VM。失效 coverage 后调度一次即可，保持同一 WS step 去重。
        ctx.request_attached_timeline_refresh(channel_id);
        Ok(())
    }
}

static POST_READ_HANDLER: PostReadHandler = PostReadHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_READ_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_READ_ACTION,
        handler: &POST_READ_HANDLER,
    }
}
