//! Sync apply 相关 Effect/StorageOp 工厂。

use crate::sync_session::EventEnvelope;
use helix_core::effect::StorageOp;
use helix_core::Effect;

/// C3：逐 sync 事件产出 mutation 事件 Effect（真源 dispatch table post.rs:1157-1448）。
pub fn sync_mutation_emits(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
) -> Vec<Effect> {
    sync_mutation_emits_with_auth(events, messages, "")
}

pub fn sync_mutation_emits_with_auth(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<Effect> {
    use crate::sync_session::EventKind;

    let empty_fields = crate::sync_session::PostFields::default();
    let mut emits = Vec::new();
    for ev in events {
        let Some(msg_id) = ev.msg_id.as_deref() else {
            continue;
        };
        match ev.kind {
            EventKind::PostUpsert => {
                if let Some(fields) = messages.get(msg_id) {
                    let update = crate::channel_write::post_updates_from_fields(
                        ev.channel_id,
                        fields,
                        auth_user_id,
                    );
                    if update.visible {
                        emits.push(super::to_effect::emit_post_received(
                            ev.channel_id,
                            ev.seq.0,
                            msg_id,
                            fields,
                        ));
                    }
                }
            }
            EventKind::PostEdit => {
                if let Some(fields) = messages.get(msg_id) {
                    emits.push(super::to_effect::emit_post_updated(
                        ev.channel_id,
                        ev.seq.0,
                        msg_id,
                        fields,
                    ));
                }
            }
            EventKind::PostRevoke => {
                emits.push(super::to_effect::emit_post_deleted(
                    ev.channel_id,
                    ev.seq.0,
                    msg_id,
                    &empty_fields,
                ));
            }
            EventKind::PostRead => {
                let fields = messages.get(msg_id).unwrap_or(&empty_fields);
                emits.push(super::to_effect::emit_channel_read_echo(
                    ev.channel_id,
                    ev.seq.0,
                    msg_id,
                    fields,
                ));
            }
            EventKind::Other(_) => {}
        }
    }
    emits
}

/// 批量 upsert EventEnvelope 列表（inline 内容路径：Snapshot / 直接事件态 / 测试帧）。
pub fn batch_upsert_events(events: &[EventEnvelope]) -> Vec<StorageOp> {
    events
        .iter()
        .map(crate::channel::event_to_upsert_op)
        .collect()
}

/// C2：批量落库 sync `events`，按 `messages` map 做 phantom / readBits / edit / revoke 分桶。
pub fn batch_upsert_events_with_messages(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
) -> Vec<StorageOp> {
    batch_upsert_events_with_messages_and_auth(events, messages, "")
}

pub fn batch_upsert_events_with_messages_and_auth(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::with_capacity(events.len());
    for ev in events {
        let msg_id = ev.msg_id.as_deref();
        match ev.kind {
            EventKind::PostUpsert => {
                if let Some(fields) = msg_id.and_then(|id| messages.get(id)) {
                    let update = crate::channel_write::post_updates_from_fields(
                        ev.channel_id,
                        fields,
                        auth_user_id,
                    );
                    if !update.visible {
                        continue;
                    }
                    let ev_with_body = crate::sync_session::EventEnvelope::new(
                        ev.channel_id,
                        ev.seq,
                        ev.kind.clone(),
                        fields.clone(),
                    );
                    ops.push(crate::channel::event_to_upsert_op(&ev_with_body));
                }
            }
            EventKind::PostEdit => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        ops.push(crate::channel::edit_content_op(id, fields));
                    }
                }
            }
            EventKind::PostRevoke => {
                if let Some(id) = msg_id {
                    ops.push(crate::channel::revoke_op(id));
                }
            }
            EventKind::PostRead => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        if !fields.read_bits.is_empty() {
                            ops.push(crate::channel::apply_read_op(id, &fields.read_bits));
                        }
                    }
                }
            }
            EventKind::Other(_) => {}
        }
    }
    ops
}

#[cfg(test)]
#[path = "sync_effects_tests.rs"]
mod tests;

/// A3/CAP-9：离线 sync 应用的可见 type1 新消息，按在线同口径落 channel 未读/lastPost。
pub fn sync_unread_bump_ops(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::new();
    for ev in events {
        if !matches!(ev.kind, EventKind::PostUpsert) {
            continue;
        }
        let Some(fields) = ev.msg_id.as_deref().and_then(|id| messages.get(id)) else {
            continue;
        };
        let upd =
            crate::channel_write::post_updates_from_fields(ev.channel_id, fields, auth_user_id);
        if !upd.visible {
            continue;
        }
        ops.push(super::to_effect::bump_channel_unread_op(&upd));
    }
    ops
}

pub fn sync_dialog_patch_plans(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<crate::dialog_patch::PendingDialogPatch> {
    use crate::sync_session::EventKind;

    let mut plans = Vec::new();
    for ev in events {
        if !matches!(ev.kind, EventKind::PostUpsert) {
            continue;
        }
        let Some(msg_id) = ev.msg_id.as_deref() else {
            continue;
        };
        let Some(fields) = messages.get(msg_id) else {
            continue;
        };
        let update =
            crate::channel_write::post_updates_from_fields(ev.channel_id, fields, auth_user_id);
        if !update.visible {
            continue;
        }
        plans.push(crate::dialog_patch::PendingDialogPatch::new(
            ev.channel_id,
            ev.seq.0,
            msg_id,
            fields,
            &update,
            "sync_events",
        ));
    }
    plans
}
