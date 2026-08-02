//! `post_read` action handler（type=6 已读位覆盖式写，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:117 handle_read` → `Message::update_fields_by_message_id`
//! 覆盖式写 `read_bits`（服务端权威单调，**非**应用层按位 OR）→ emit `im:post:read`。
//!
//! wire：`postId` / `authorUserId` / `readerUserId` / `snapshotId` / `memberIds` /
//! `readMap`(→read_bits) / `createAt` / `updateAt`。
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
//! ∴ post_read 走**专用相关持久直路**：`apply_read_op`（覆盖 read_bits）先经 `PersistOk`，
//! 成功后才释放当前 viewer 的 MessageV3 已读终态；**不经 gate**。守 HX-C005（数据在手
//! 不回读：read_bits 取帧内 readMap，不回查 DB）。
//!
//! content event（`post` / `post_update` / `post_revoke`）仍走 gate 严格 +1（那是带序的消息事件，
//! 序列化对消息正确）——本改**只**让幂等覆盖类 read 旁路，gate 对消息事件的 event_seq 校验不动。

use helix_core::{Effect, EffectSink};
use serde_json::Value;

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

    /// 按字段形状分流 G06b ID 增量或 G14 位图持久屏障。
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

        // G06b 只接受无旧成员投影字段的四字段 delta；缺字段、未知字段或 forbidden 字段均 no-op。
        if has_minimal_identity_fields(data) && !has_legacy_member_fields(data) {
            let Some((snapshot_id, reader_id)) = minimal_post_read_fields(data, &post_id) else {
                return Ok(());
            };
            let key = (
                channel_id,
                post_id.clone(),
                snapshot_id.clone(),
                reader_id.clone(),
            );
            if !ctx.state.seen_post_read_deltas.insert(key) {
                return Ok(());
            }
            let event = crate::event::read::post(serde_json::json!({
                "channelId": channel_id.as_str(),
                "postId": post_id,
                "snapshotId": snapshot_id,
                "readerUserId": reader_id,
            }))?;
            out.push(event.into_effect());
            return Ok(());
        }
        if !has_legacy_projection_fields(data) {
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
        let reader_id = data
            .get("readerUserId")
            .or_else(|| data.get("reader_user_id"))
            .or_else(|| data.get("userId"))
            .or_else(|| data.get("user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let author_user_id = data
            .get("authorUserId")
            .or_else(|| data.get("author_user_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let snapshot_id = data
            .get("snapshotId")
            .or_else(|| data.get("snapshot_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
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

/// 判断帧是否明确携带 G14 位图/成员投影字段，避免混合帧误走 G06b 分支。
fn has_legacy_projection_fields(data: &Value) -> bool {
    ["readMap", "read_map", "readBits", "read_bits"]
        .iter()
        .any(|key| data.get(*key).is_some())
}

/// 判断帧是否包含 G14 成员快照字段；这些字段优先进入旧持久化链。
fn has_legacy_member_fields(data: &Value) -> bool {
    let has_read_map = ["readMap", "read_map"]
        .iter()
        .any(|key| data.get(*key).is_some());
    let has_member_ids = ["memberIds", "member_ids"]
        .iter()
        .any(|key| data.get(*key).is_some());
    has_read_map && has_member_ids
}

/// 判断帧是否出现 G06b 的 snapshot/reader 身份字段。
fn has_minimal_identity_fields(data: &Value) -> bool {
    [
        "snapshotId",
        "snapshot_id",
        "readerUserId",
        "reader_user_id",
    ]
    .iter()
    .any(|key| data.get(*key).is_some())
}

/// 解析严格四字段 G06b delta；字段集合超出 canonical 形状时 fail closed。
fn minimal_post_read_fields(data: &Value, post_id: &str) -> Option<(String, String)> {
    crate::state::ServerId::from_str(post_id)?;
    let object = data.as_object()?;
    const ALLOWED: [&str; 4] = ["channelId", "postId", "snapshotId", "readerUserId"];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return None;
    }
    let snapshot_id = object
        .get("snapshotId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let reader_id = object
        .get("readerUserId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    Some((snapshot_id, reader_id))
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
