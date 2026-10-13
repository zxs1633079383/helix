//! `post_update` action handler（type=2 单条消息编辑，spec §S1/§S4）。
//!
//! 行为真源：现网 `handlers/post.rs:171 handle_update` → `message_service::handle_post_update`
//! （event_type=2 / "post_edited"）→ emit `im:post:updated` + 经 `on_ws_event` 推 cursor。
//!
//! wire：`data` 即 post 对象（顶层 `id` / `channelId` / `message` / `props` / `event_seq`）。
//! 落库：普通 edit 走 `edit_content_op`；G-06 quickReply 走独立单列写与 cursor 原子提交，
//! PersistOk 后再读 durable row。G-14a template props 走同一 MessageV3 连续性链。
//!
//! BLOCKING-2：cursor 推进经 MessageV3 gate（严格 +1 / drop / buffer+1s-gate），与 post handler
//! 共用 `admit_message_v3_post`。普通 type2 的内容 patch 与 cursor 进入 correlated
//! `PersistAtomic`，matching PersistOk 后才释放 `im:post:updated` 并继续下一条 buffer 事件。

use helix_core::EffectSink;

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
        apply_post_update(ctx, frame, data, out)
    }
}

/// 单条 post 编辑——经 MessageV3 gate 与 correlated 原子屏障推进 cursor（type2，BLOCKING-2）。
///
/// `post` = 单个 post JSON 对象。普通 edit 与 G-06 reaction/G-07 urgent 共用 MessageV3
/// 连续性 buffer，严格 +1 后才进入 correlated PersistAtomic。
/// channel_id 缺 / 未注册 → no-op（边界零信任，与 post handler ingest 门控同口径）。
pub(super) fn apply_post_update(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    data: &serde_json::Value,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let post = data.get("post").unwrap_or(data);
    let Some(channel_id) = post
        .get("channelId")
        .or_else(|| post.get("channel_id"))
        .or_else(|| data.get("channelId"))
        .or_else(|| data.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return Ok(());
    };
    let msg_id = post
        .get("id")
        .or_else(|| post.get("postId"))
        .or_else(|| post.get("post_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // type=2 fields 取自该 post 子树；经 gate ingest（落库 patch + emit + cursor 一并 kind-aware）。
    // 直接检查借用 wire 值，避免序列化后再次解析；错误类型必须在 gate/游标提交前拒绝。
    if let Some(expedite) = post.get("expediteMap").or_else(|| post.get("expedite_map")) {
        if !expedite.is_object()
            && !expedite.is_null()
            && !expedite.as_str().is_some_and(str::is_empty)
        {
            return Err(ImError::Parse(
                "post_update expediteMap must be an object".into(),
            ));
        }
    }
    let fields = extract_post_fields(post);
    // CATEGORY_CHAIN authority must be rejected before gate admission, otherwise a malformed
    // edit can become an admitted event without a correlated durable commit.
    if crate::category_chain::post::validate_fields(&fields).is_err() {
        return Ok(());
    }
    // 投票/平均分只记录结构形状，验证完整帧是否在 Helix parser 前仍保留；不输出正文、成员姓名或凭证。
    if matches!(fields.msg_type.as_str(), "VOTE" | "AVERAGE_SCORE") {
        let props = serde_json::from_str::<serde_json::Value>(&fields.props).unwrap_or_default();
        let entity_key = if fields.msg_type == "VOTE" {
            "vote"
        } else {
            "averageScore"
        };
        let entity = props.get(entity_key).and_then(serde_json::Value::as_object);
        let collection_len = |key: &str| {
            entity
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len)
        };
        tracing::info!(
            target: "helix_im::engagement_projection",
            hop = "post_update.parsed",
            msg_id = fields.id.as_str(),
            post_type = fields.msg_type.as_str(),
            props_bytes = fields.props.len(),
            props_has_entity = entity.is_some(),
            items_or_participants = collection_len(if fields.msg_type == "VOTE" { "items" } else { "participants" }),
            options_or_members = collection_len(if fields.msg_type == "VOTE" { "options" } else { "members" }),
            state = entity
                .and_then(|value| value.get("state"))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default(),
            "engagement post_update shape parsed by Helix"
        );
    }
    let Some(target_seq) = frame.event_seq() else {
        return Ok(());
    };
    if ctx.auth_user_id.is_empty() || i64::try_from(target_seq.0).is_err() {
        return Ok(());
    }
    if fields.msg_type == "CATEGORY_CHAIN" {
        let event = crate::sync_session::EventEnvelope::new(
            channel_id,
            target_seq,
            EventKind::PostEdit,
            fields,
        )
        .with_msg_id(msg_id)
        .with_viewer_user_id(ctx.auth_user_id);
        let Some(channel) = ctx.state.channels.get_mut(&channel_id) else {
            return Ok(());
        };
        if let Ok(Some(event)) = channel.admit_message_v3_post(event, out) {
            let corr = ctx.alloc_corr();
            crate::category_chain::post::queue_edit(ctx.state, corr, event, out);
        }
        super::gate::trigger_backfill_if_large_gap(ctx, channel_id, target_seq, out);
        return Ok(());
    }
    let has_template_confirmation =
        crate::event::post::has_template_confirmation(fields.props.as_str());
    if should_use_message_v3_compact_update(&fields, has_template_confirmation) {
        let event = crate::sync_session::EventEnvelope::new(
            channel_id,
            target_seq,
            EventKind::PostEdit,
            fields,
        )
        .with_msg_id(msg_id)
        .with_viewer_user_id(ctx.auth_user_id);
        let Some(channel) = ctx.state.channels.get_mut(&channel_id) else {
            return Ok(());
        };
        let Ok(admitted) = channel.admit_message_v3_post(event, out) else {
            return Ok(());
        };
        if let Some(event) = admitted {
            let corr = ctx.alloc_corr();
            if has_quick_reply_items(event.fields.quick_reply.as_str()) {
                crate::port_reply::message_v3_reaction::queue_commit(ctx.state, corr, event, out);
            } else if !event.fields.expedite_map.is_empty() {
                crate::port_reply::message_v3_urgent::queue_commit(ctx.state, corr, event, out);
            } else {
                crate::port_reply::message_v3_template::queue_commit(ctx.state, corr, event, out);
            }
        }
        super::gate::trigger_backfill_if_large_gap(ctx, channel_id, target_seq, out);
        return Ok(());
    }
    let event = crate::sync_session::EventEnvelope::new(
        channel_id,
        target_seq,
        EventKind::PostEdit,
        fields,
    )
    .with_msg_id(msg_id)
    .with_viewer_user_id(ctx.auth_user_id);
    let Some(channel) = ctx.state.channels.get_mut(&channel_id) else {
        return Ok(());
    };
    let Ok(admitted) = channel.admit_message_v3_post(event, out) else {
        return Ok(());
    };
    if let Some(event) = admitted {
        let corr = ctx.alloc_corr();
        crate::port_reply::message_v3_post::queue_post_update_commit(
            ctx.state,
            ctx.auth_user_id,
            corr,
            event,
            out,
        );
    }
    super::gate::trigger_backfill_if_large_gap(ctx, channel_id, target_seq, out);
    Ok(())
}

/// 判断 post_update 是否应走 reaction/urgent/template 的精简持久化路径。
///
/// 投票与平均分虽然可能携带空的 `quickReply` wire 字段，但它们必须保留完整
/// `type/props/items|participants` 快照，不能被 G-06 的稀疏回读事件覆盖。
fn should_use_message_v3_compact_update(
    fields: &crate::sync_session::PostFields,
    has_template_confirmation: bool,
) -> bool {
    let is_engagement = fields.msg_type.eq_ignore_ascii_case("VOTE")
        || fields.msg_type.eq_ignore_ascii_case("AVERAGE_SCORE");
    // 明确的空回应必须完整写回；同时携带的加急/模板元数据不能吞掉取消。
    !is_engagement
        && fields.quick_reply.trim() != "[]"
        && (has_quick_reply_items(fields.quick_reply.as_str())
            || !fields.expedite_map.is_empty()
            || has_template_confirmation)
}

#[cfg(test)]
mod tests {
    use super::should_use_message_v3_compact_update;
    use crate::ws::parser::extract_post_fields;
    use serde_json::json;

    /// 空回应即使带加急/模板元数据也不能进入只写其中一列的稀疏提交。
    #[test]
    fn empty_reaction_with_urgent_metadata_keeps_full_snapshot_path() {
        for urgent in [json!({}), json!({"member": {"status": 1}})] {
            let fields = extract_post_fields(
                &json!({"type": "TEXT", "quickReply": [], "expediteMap": urgent}),
            );
            assert!(!should_use_message_v3_compact_update(&fields, false));
            assert!(!should_use_message_v3_compact_update(&fields, true));
        }
        let urgent_only =
            extract_post_fields(&json!({"type": "TEXT", "expediteMap": {"member": {"status": 1}}}));
        assert!(should_use_message_v3_compact_update(&urgent_only, false));
    }

    /// 投票帧的空 quickReply 不能触发稀疏 reaction 投影。
    #[test]
    fn vote_update_with_empty_quick_reply_keeps_full_snapshot_path() {
        let fields = extract_post_fields(&json!({
            "type": "VOTE",
            "quickReply": [],
            "props": {"vote": {"items": [{"userId": "member"}]}}
        }));

        assert!(!should_use_message_v3_compact_update(&fields, false));
    }

    /// 平均分帧同样不能因为空 quickReply 被降级成稀疏投影。
    #[test]
    fn average_score_update_with_empty_quick_reply_keeps_full_snapshot_path() {
        let fields = extract_post_fields(&json!({
            "type": "AVERAGE_SCORE",
            "quickReply": [],
            "props": {"averageScore": {"participants": [{"userId": "member"}]}}
        }));

        assert!(!should_use_message_v3_compact_update(&fields, false));
    }
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

/// 只有含有实际 reaction 项时才进入 quick-reply 专用持久化路径；空数组属于完整 post 快照。
pub(crate) fn has_quick_reply_items(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(items)) => !items.is_empty(),
        Ok(serde_json::Value::Null) => false,
        Ok(_) | Err(_) => true,
    }
}
