//! `post_read` action handler（type=6 已读位覆盖式写，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:117 handle_read` → `Message::update_fields_by_message_id`
//! 覆盖式写 `read_bits`（服务端权威单调，**非**应用层按位 OR）→ PersistOk 后 emit viewer-relative projection。
//!
//! wire：`channelId` / `postId` / `authorUserId` / `readerUserId` / `snapshotId` /
//! `memberIds` / `readMap`(→read_bits) / `createAt` / `updateAt`。
//!
//! ## 为什么 post_read 绕开 channel gate（不走 event_seq 严格 +1 排序）
//!
//! `read_bits`（`readMap`）是服务端权威**全量已读位图**——幂等覆盖、order-independent、与消息
//! 序列**解耦**：
//!   - 后端 `post_read` 广播帧**不携带 `event_seq`**（read 不产生 sequenced channel_event；
//!     Go `post_read` authority 同时携带 `authorUserId/readerUserId/snapshotId/memberIds`，
//!     这些字段只在 Go→Helix 边界可见，绝不外发给 Angular；
//!   - 走 `gate_ingest_content_event` 会因 `frame.event_seq() == None` 直接 no-op
//!     → 永不 emit / 永不落库（issue #14 L2 read-receipt ②④ 永红根因）；
//!   - read 是对**既存** message 行 `read_bits` 列的覆盖写（O(1) `UPDATE … WHERE id`），不需要
//!     `+1` cursor 排序、不参与 `flush_contiguous`、**不**推进 cursor——cursor 是消息序列水位，
//!     把无序的旁路已读回执塞进 gate 推进它会污染同步基线（违 spec §S6 cursor 单调语义）。
//!
//! ∴ post_read 走**专用相关持久直路**：`apply_read_op`（覆盖 read_bits）先经 `PersistOk`，
//! 成功后才释放当前 viewer 的 MessageV3 已读终态；**不经 gate**。Go 发送的完整 authority
//! 是唯一可接受输入，Helix 在本地 PersistOk 后将其归一成绝对 read/unread projection；
//! 四字段 reader delta 不再生成任何事件。
//!
//! content event（`post` / `post_update` / `post_revoke`）仍走 gate 严格 +1（那是带序的消息事件，
//! 序列化对消息正确）——本改**只**让幂等覆盖类 read 旁路，gate 对消息事件的 event_seq 校验不动。

use helix_core::{Effect, EffectSink};
use serde_json::Value;
use std::collections::HashSet;

use crate::error::ImError;
use crate::state::ChannelId;
use crate::ws::parser::extract_post_fields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POST_READ_ACTION: &str = "post_read";

struct PostReadHandler;

impl WsMessageHandler for PostReadHandler {
    /// 声明当前 handler 唯一消费的服务端权威 action。
    fn action(&self) -> &'static str {
        POST_READ_ACTION
    }

    /// 只接受 Go 的完整回执 authority，并在 matching PersistOk 后释放绝对投影。
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

        // 四字段 delta 已废弃：缺少 member snapshot/readMap 时 fail-closed，不向 Angular 发增量。
        if !has_receipt_bit_fields(data) || !has_receipt_member_fields(data) {
            return Ok(());
        }
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

        if let Some((committed_revision, committed_bits)) =
            ctx.state.committed_post_reads.get(post_id.as_str())
        {
            if (*committed_revision == receipt_revision && committed_bits == &fields.read_bits)
                || (receipt_revision > 0
                    && *committed_revision > 0
                    && receipt_revision < *committed_revision)
            {
                return Ok(());
            }
        }
        let Some(reader_id) = data
            .get("readerUserId")
            .or_else(|| data.get("reader_user_id"))
            .or_else(|| data.get("userId"))
            .or_else(|| data.get("user_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let Some(author_user_id) = data
            .get("authorUserId")
            .or_else(|| data.get("author_user_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let Some(snapshot_id) = data
            .get("snapshotId")
            .or_else(|| data.get("snapshot_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let member_ids: Vec<String> = data
            .get("memberIds")
            .or_else(|| data.get("member_ids"))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let mut member_set = HashSet::with_capacity(member_ids.len());
        let valid_receipt_shape = !member_ids.is_empty()
            && member_ids
                .iter()
                .all(|member_id| !member_id.is_empty() && member_id.trim() == member_id)
            && member_ids.iter().all(|member_id| member_set.insert(member_id))
            && fields.read_bits.len() == member_ids.len()
            && fields
                .read_bits
                .bytes()
                .all(|bit| bit == b'0' || bit == b'1');
        if !valid_receipt_shape {
            return Ok(());
        }
        let terminal_event = match crate::event::read::post_for_viewer(
            channel_id.as_str(),
            post_id.as_str(),
            author_user_id,
            reader_id,
            snapshot_id,
            &member_ids,
            fields.read_bits.as_str(),
            receipt_revision,
            ctx.auth_user_id,
        ) {
            Ok(event) => event.into_bytes(),
            Err(_) => return Ok(()),
        };
        let read_op = crate::channel::apply_read_op(&post_id, fields.read_bits.as_str());
        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::PostReadPersist {
                channel_id,
                message_id: post_id,
                receipt_revision,
                read_bits: fields.read_bits,
                terminal_event,
            },
        );
        // read_bits 是旁路权威事实，不推进 channel cursor；但必须等待相关写回执后再对 renderer 可见。
        out.push(Effect::Persist {
            corr,
            ops: vec![read_op],
        });
        Ok(())
    }
}

/// 判断帧是否明确携带 Go/Helix receipt 位图字段。
fn has_receipt_bit_fields(data: &Value) -> bool {
    ["readMap", "read_map", "readBits", "read_bits"]
        .iter()
        .any(|key| data.get(*key).is_some())
}

/// 判断帧是否同时携带创建时成员快照；缺失时禁止生成任何 receipt projection。
fn has_receipt_member_fields(data: &Value) -> bool {
    let has_read_projection = ["readMap", "read_map", "readBits", "read_bits"]
        .iter()
        .any(|key| data.get(*key).is_some());
    let has_member_ids = ["memberIds", "member_ids"]
        .iter()
        .any(|key| data.get(*key).is_some());
    has_read_projection && has_member_ids
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
