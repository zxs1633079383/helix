//! Event-kind render projection and dialog patch collection.

use crate::sync_session::EventEnvelope;
use helix_core::Effect;

/// kind-aware 前端通知 Effect（gate apply/flush 用，BLOCKING-2）。
///
/// 真源 apply_event_with_messages 按 type emit 不同 im:post:* 事件。
/// post=received / edit=updated / revoke=deleted / read=read；未知类型按 received 兜底。
///
/// **spec06 FixB**：传 `&ev.fields`（**在手数据**，与本事件正落库的 `event_to_storage_op(ev)` 同一份
/// owned `PostFields`）给 fat emit 工厂 → 携完整可渲染 MessageItemData，前端零回读直渲。零额外读、零再解析。
pub fn emit_for_kind(ev: &EventEnvelope) -> Effect {
    use crate::sync_session::EventKind;
    let msg_id = ev.msg_id.as_deref().unwrap_or("");
    let seq = ev.seq.0;
    let f = &ev.fields;
    match ev.kind {
        EventKind::PostUpsert => crate::acl::to_effect::emit_post_received_for_viewer(
            ev.id,
            seq,
            msg_id,
            f,
            ev.viewer_user_id.as_str(),
        ),
        EventKind::PostEdit => crate::acl::to_effect::emit_post_updated_for_viewer(
            ev.id,
            seq,
            msg_id,
            f,
            ev.viewer_user_id.as_str(),
        ),
        EventKind::PostRevoke => crate::acl::to_effect::emit_post_deleted_for_viewer(
            ev.id,
            seq,
            msg_id,
            f,
            ev.event_id.as_str(),
            ev.actor_id.as_str(),
            ev.occurred_at,
            ev.viewer_user_id.as_str(),
        ),
        EventKind::PostRead => crate::acl::to_effect::emit_post_read_for_viewer(
            ev.id,
            seq,
            msg_id,
            f,
            ev.viewer_user_id.as_str(),
        ),
        // type7 is never a post projection. Legacy WS input rejects it and sync::flow publishes
        // the closed dialog Frame only after PersistAtomic succeeds.
        EventKind::ChannelTerminalClosed => Effect::PersistFire { ops: Vec::new() },
        EventKind::Other(_) => crate::acl::to_effect::emit_post_received(ev.id, seq, msg_id, f),
    }
}

pub(super) fn collect_dialog_patch_for_post(
    ev: &EventEnvelope,
    source: &str,
    out: &mut Vec<crate::dialog_patch::PendingDialogPatch>,
) {
    use crate::sync_session::EventKind;
    if !matches!(ev.kind, EventKind::PostUpsert) {
        return;
    }
    let Some(update) = ev.unread_bump.as_ref() else {
        return;
    };
    if !update.visible {
        return;
    }
    let msg_id = ev
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(ev.fields.id.as_str());
    out.push(
        crate::dialog_patch::PendingDialogPatch::new(
            ev.id, ev.seq.0, msg_id, &ev.fields, update, source,
        )
        .with_causation_id(ev.causation_id.clone()),
    );
}

pub(super) fn visible_post_for_storage(ev: &EventEnvelope) -> bool {
    use crate::sync_session::EventKind;
    match ev.kind {
        EventKind::PostUpsert => ev.unread_bump.as_ref().map(|u| u.visible).unwrap_or(true),
        EventKind::ChannelTerminalClosed => false,
        EventKind::PostEdit | EventKind::PostRevoke | EventKind::PostRead | EventKind::Other(_) => {
            true
        }
    }
}
