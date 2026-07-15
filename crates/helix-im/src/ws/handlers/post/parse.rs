use helix_core::EffectSink;

use crate::state::{ChannelId, Seq};
use crate::sync_session::{EventEnvelope, EventKind};

use super::WsFrame;

/// 解析真 Go `action=="post"` 在线广播帧；缺频道或序号时拒绝推进 cursor。
pub(super) fn parse_post_frame(frame: &WsFrame) -> Option<EventEnvelope> {
    let data = frame.data()?;
    let post = data.get("post").unwrap_or(data);
    let channel_id = data
        .get("channelId")
        .or_else(|| data.get("channel_id"))
        .or_else(|| post.get("channelId"))
        .or_else(|| post.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)?;
    let seq = Seq(data
        .get("event_seq")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            post.get("props")
                .and_then(|props| props.get("channel_event_seq"))
                .and_then(serde_json::Value::as_u64)
        })
        .or_else(|| {
            frame
                .root()
                .get("channel_event_seq")
                .and_then(serde_json::Value::as_u64)
        })?);
    let fields = crate::ws::parser::extract_post_fields(data);
    let msg_id = post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some(EventEnvelope::new(channel_id, seq, EventKind::PostUpsert, fields).with_msg_id(msg_id))
}

pub(super) fn parse_post_data(frame: &WsFrame) -> Option<&serde_json::Value> {
    let data = frame.data()?;
    Some(data.get("post").unwrap_or(data))
}

/// 无序回声只发渲染投影，不推进 channel cursor。
pub(super) fn emit_unsequenced_post(
    frame: &WsFrame,
    viewer_user_id: &str,
    out: &mut EffectSink,
) -> Option<ChannelId> {
    let Some(data) = frame.data() else {
        return None;
    };
    let post = data.get("post").unwrap_or(data);
    let Some(channel_id) = post
        .get("channelId")
        .or_else(|| post.get("channel_id"))
        .or_else(|| data.get("channelId"))
        .or_else(|| data.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return None;
    };
    let fields = crate::ws::parser::extract_post_fields(post);
    let msg_id = post
        .get("id")
        .or_else(|| post.get("postId"))
        .or_else(|| post.get("post_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fields.id.as_str());
    out.push(crate::acl::to_effect::emit_post_received_for_viewer(
        channel_id,
        0,
        msg_id,
        &fields,
        viewer_user_id,
    ));
    Some(channel_id)
}
