use crate::state::ChannelId;
use crate::sync_session::EventEnvelope;

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
    // NOTICE content is the authoritative full target snapshot; never rebuild it from defaults.
    let mut fields = crate::ws::parser::extract_post_fields(&target_post);
    if fields.id.is_empty() {
        fields.id = post_id.to_string();
    }
    if fields.channel_id.is_empty() {
        fields.channel_id = channel_id.as_str().to_string();
    }
    // A fat timeline projection without a positive create_at would recreate the anchor bug.
    if fields.create_at <= 0 {
        return None;
    }
    let mut target_props = serde_json::from_str::<serde_json::Value>(&fields.props)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(props) = target_props.as_object_mut() {
        props.insert("pinned".to_string(), serde_json::Value::Bool(pinned));
    }
    fields.props = target_props.to_string();
    Some((
        helix_core::Effect::PersistFire {
            ops: vec![crate::channel::pin_state_op(post_id, pinned)],
        },
        crate::acl::to_effect::emit_post_updated(channel_id, ev.seq.0, post_id, &fields),
    ))
}
