use helix_core::Effect;

use crate::state::Seq;
use crate::sync_session::EventEnvelope;

use super::super::WsFrame;

pub(super) fn post_channel_update(
    frame: &WsFrame,
    ev: &EventEnvelope,
    auth_user_id: &str,
) -> Option<crate::channel_write::PostChannelUpdate> {
    let data = parse_post_data(frame)?;
    Some(crate::channel_write::post_updates(
        ev.channel_id,
        data,
        auth_user_id,
        ev.fields.create_at,
    ))
}

pub(super) fn echo_channel_change_effect(
    update: Option<crate::channel_write::PostChannelUpdate>,
    ev: &EventEnvelope,
) -> Option<Effect> {
    update.map(|mut upd| {
        upd.unread_delta = 0;
        upd.unread_post_id = None;
        let msg_id = ev
            .msg_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(ev.fields.id.as_str());
        crate::acl::to_effect::emit_channel_changed_by_post(&upd, seq(ev), msg_id)
    })
}

fn seq(ev: &EventEnvelope) -> u64 {
    let Seq(value) = ev.seq;
    value
}

fn parse_post_data(frame: &WsFrame) -> Option<&serde_json::Value> {
    let data = frame.data()?;
    Some(data.get("post").unwrap_or(data))
}
