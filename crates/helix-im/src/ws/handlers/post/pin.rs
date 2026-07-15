use crate::state::ChannelId;
use crate::sync_session::{EventEnvelope, PostFields};

/// 将 Go 的 post pin 通知归一为目标消息 pinned patch。
pub(super) fn post_pin_effect_from_notice(
    ev: &EventEnvelope,
) -> Option<(helix_core::Effect, helix_core::Effect)> {
    if !ev.fields.msg_type.eq_ignore_ascii_case("NOTICE") || ev.fields.props.is_empty() {
        return None;
    }
    let props: serde_json::Value = serde_json::from_str(&ev.fields.props).ok()?;
    let prop_type = props.get("type").and_then(serde_json::Value::as_str)?;
    let pinned = match prop_type {
        "addPostPin" => true,
        "removePostPin" => false,
        _ => return None,
    };
    let target_post: serde_json::Value =
        serde_json::from_str(props.get("content").and_then(serde_json::Value::as_str)?).ok()?;
    let post_id = target_post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())?;
    let channel_id = target_post
        .get("channelId")
        .or_else(|| target_post.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .unwrap_or(ev.channel_id);
    let fields = PostFields {
        id: post_id.to_string(),
        channel_id: channel_id.as_str().to_string(),
        props: serde_json::json!({ "pinned": pinned }).to_string(),
        ..PostFields::default()
    };
    Some((
        helix_core::Effect::PersistFire {
            ops: vec![crate::channel::pin_state_op(post_id, pinned)],
        },
        crate::acl::to_effect::emit_post_updated(channel_id, ev.seq.0, post_id, &fields),
    ))
}
