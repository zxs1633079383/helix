//! Sync apply 相关 Effect/StorageOp 工厂。

use crate::sync_session::EventEnvelope;
use helix_core::effect::StorageOp;
use helix_core::Effect;

/// 判断 type=2 是否只是接龙声明公告的历史回放。
///
/// 接龙声明的 canonical 事实由 `synced_projection` 写入 `chain_*` 表并通过
/// `chainProjection` 事件投影；它不是普通消息编辑。新租户库没有对应 `message` 行时，
/// 若继续生成全量 type=2 UPDATE，会触发 host 的严格 0 行回滚，连带丢失接龙投影。
fn is_chain_declaration(fields: &crate::sync_session::PostFields) -> bool {
    if !fields.msg_type.eq_ignore_ascii_case("ANNOUNCEMENT") {
        return false;
    }
    let Ok(props) = serde_json::from_str::<serde_json::Value>(&fields.props) else {
        return false;
    };
    props.get("chain").is_some()
        || props
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("chain"))
}

/// C3：逐 sync 事件产出 mutation 事件 Effect（真源 dispatch table post.rs:1157-1448）。
pub fn sync_mutation_emits(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
) -> Vec<Effect> {
    sync_mutation_emits_with_auth(events, messages, "")
}

/// 按 sync authority 顺序构造提交后事件，type3 只发布 MessageV3 revoke。
pub fn sync_mutation_emits_with_auth(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<Effect> {
    use crate::sync_session::EventKind;

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
                        emits.push(super::to_effect::emit_post_received_for_viewer(
                            ev.channel_id,
                            ev.seq.0,
                            msg_id,
                            fields,
                            auth_user_id,
                        ));
                    }
                }
            }
            EventKind::PostEdit => {
                if let Some(fields) = messages.get(msg_id) {
                    emits.push(super::to_effect::emit_post_updated_for_viewer(
                        ev.channel_id,
                        ev.seq.0,
                        msg_id,
                        fields,
                        auth_user_id,
                    ));
                }
            }
            EventKind::PostRevoke => {
                if let Ok(event) = crate::event::post::revoke_from_sync_authority(ev) {
                    emits.push(event.into_effect());
                }
            }
            EventKind::PostRead => {
                // 无消息快照无法证明 viewer 权限，read replay 必须 fail-closed。
                let Some(fields) = messages.get(msg_id) else {
                    continue;
                };
                if !crate::channel_write::post_updates_from_fields(
                    ev.channel_id,
                    fields,
                    auth_user_id,
                )
                .visible
                {
                    continue;
                }
                emits.push(super::to_effect::emit_channel_read_echo_for_viewer(
                    ev.channel_id,
                    ev.seq.0,
                    msg_id,
                    fields,
                    auth_user_id,
                ));
                // UC13 离线 sender 回执必须同时由三项权威事实证明：事件锚定 post、actorId
                // 锚定 reader、messages 快照给出持久化 readBits。缺一项绝不猜 reader
                // 或伪造 post:read。
                if !ev.actor_id.is_empty() && !fields.read_bits.is_empty() {
                    emits.push(super::to_effect::emit_sync_post_read_for_viewer(
                        ev.channel_id,
                        ev.seq.0,
                        msg_id,
                        fields,
                        ev.actor_id.as_str(),
                        ev.occurred_at,
                        auth_user_id,
                    ));
                }
            }
            EventKind::ChannelTerminalClosed => {}
            EventKind::Other(_) => {}
        }
    }
    emits
}

/// 收集仅能在 sync 原子提交成功后结算的本端发送事实。
///
/// 只接受可见的 `PostUpsert`、非空 temporaryId 与合法 26 字符 server id；因此普通历史
/// 回放、phantom 事件和他人消息不会越权覆盖本地发送态。
pub(crate) fn pending_send_reconciliations(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<crate::state::PendingSendReconciliation> {
    use crate::state::{PendingSendReconciliation, ServerId, TemporaryId};
    use crate::sync_session::EventKind;

    events
        .iter()
        .filter_map(|event| {
            if !matches!(event.kind, EventKind::PostUpsert) {
                return None;
            }
            let fields = event
                .msg_id
                .as_deref()
                .and_then(|msg_id| messages.get(msg_id))?;
            if fields.temporary_id.is_empty()
                || !crate::channel_write::post_updates_from_fields(
                    event.channel_id,
                    fields,
                    auth_user_id,
                )
                .visible
            {
                return None;
            }
            let server_id = ServerId::from_str(fields.id.as_str())?;
            Some(PendingSendReconciliation {
                temporary_id: TemporaryId(fields.temporary_id.clone()),
                server_id,
            })
        })
        .collect()
}

/// 批量 upsert EventEnvelope 列表（inline 内容路径：Snapshot / 直接事件态 / 测试帧）。
pub fn batch_upsert_events(events: &[EventEnvelope]) -> Vec<StorageOp> {
    events
        .iter()
        .map(crate::channel::event_to_upsert_op)
        .collect()
}

/// 按真实 viewer 身份编译 Sync events 的 phantom/readBits/edit/revoke 与 type1 写集。
pub fn batch_upsert_events_with_messages_and_auth(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<StorageOp> {
    batch_upsert_events_with_messages_and_auth_observed(
        events,
        messages,
        auth_user_id,
        SyncApplyMode::LiveRecovery,
        None,
    )
}

/// 标记实时恢复与 Hydration 历史恢复；两者复用写集，业务事件释放策略由上层决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyncApplyMode {
    LiveRecovery,
    HydrationHistory,
}

/// C2 sync 落库编译器；可选观测上下文只影响诊断日志，不改变 StorageOp 语义。
pub(crate) fn batch_upsert_events_with_messages_and_auth_observed(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
    _mode: SyncApplyMode,
    observation: Option<&crate::sync::observability::SyncObservation>,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::with_capacity(events.len());
    for ev in events {
        let msg_id = ev.msg_id.as_deref();
        let operation_id = observation
            .map(|context| format!("sync:{}:{}:{}", context.corr, ev.seq.0, ev.kind.type_num()));
        match ev.kind {
            EventKind::PostUpsert => {
                if let Some(fields) = msg_id.and_then(|id| messages.get(id)) {
                    let update = crate::channel_write::post_updates_from_fields(
                        ev.channel_id,
                        fields,
                        auth_user_id,
                    );
                    if !update.visible {
                        if let Some(context) = observation
                            .filter(|context| context.target_matches(msg_id, Some(fields)))
                        {
                            tracing::info!(
                                hop = "sync.event.skipped",
                                corr = context.corr,
                                track_id = context.track_id.as_str(),
                                channel_id = ev.channel_id.as_str(),
                                event_seq = ev.seq.0,
                                event_type = ev.kind.type_num(),
                                msg_id = msg_id.unwrap_or_default(),
                                source = context.source,
                                operation_id = operation_id.as_deref().unwrap_or_default(),
                                reason = "invisible",
                                "type=1 消息因 viewer 可见性未准备落库"
                            );
                        }
                        continue;
                    }
                    let ev_with_body = crate::sync_session::EventEnvelope::new(
                        ev.channel_id,
                        ev.seq,
                        ev.kind.clone(),
                        fields.clone(),
                    );
                    let type1_ops = crate::channel_write::message_v3_post_mutation_ops(
                        &ev_with_body,
                        auth_user_id,
                        update.last_post.clone(),
                        crate::channel_write::PostUpsertSource::SyncSnapshot,
                    );
                    if type1_ops.is_empty() {
                        continue;
                    }
                    if let Some(context) =
                        observation.filter(|context| context.target_matches(msg_id, Some(fields)))
                    {
                        let meta = crate::sync::observability::storage_op_metadata(
                            type1_ops
                                .first()
                                .expect("type=1 compiler must contain the message upsert"),
                        );
                        let input_meta = crate::sync::observability::post_fields_metadata(fields);
                        tracing::info!(
                            hop = "sync.type1.upsert",
                            corr = context.corr,
                            track_id = context.track_id.as_str(),
                            channel_id = ev.channel_id.as_str(),
                            event_seq = ev.seq.0,
                            event_type = ev.kind.type_num(),
                            msg_id = msg_id.unwrap_or_default(),
                            source = context.source,
                            operation_id = operation_id.as_deref().unwrap_or_default(),
                            message_map_hit = true,
                            temporary_id = fields.temporary_id.as_str(),
                            post_id = fields.id.as_str(),
                            operation = "BatchUpsert",
                            conflict_key = "temporary_id",
                            field_presence = %input_meta["field_presence"],
                            field_lengths = %input_meta["field_lengths"],
                            field_hashes = %input_meta["field_hashes"],
                            field_meta = %meta["patch_field_meta"],
                            "type=1 完整消息输入已准备落库"
                        );
                    }
                    ops.extend(type1_ops);
                } else if let Some(context) =
                    observation.filter(|context| context.target_matches(msg_id, None))
                {
                    tracing::info!(
                        hop = "sync.event.skipped",
                        corr = context.corr,
                        track_id = context.track_id.as_str(),
                        channel_id = ev.channel_id.as_str(),
                        event_seq = ev.seq.0,
                        event_type = ev.kind.type_num(),
                        msg_id = msg_id.unwrap_or_default(),
                        source = context.source,
                        operation_id = operation_id.as_deref().unwrap_or_default(),
                        message_map_hit = false,
                        reason = "message_map_miss",
                        "type=1 消息未命中 messages map，保留 phantom 语义"
                    );
                }
            }
            EventKind::PostEdit => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        if is_chain_declaration(fields) {
                            if let Some(context) = observation
                                .filter(|context| context.target_matches(Some(id), Some(fields)))
                            {
                                tracing::info!(
                                    hop = "sync.event.skipped",
                                    corr = context.corr,
                                    track_id = context.track_id.as_str(),
                                    channel_id = ev.channel_id.as_str(),
                                    event_seq = ev.seq.0,
                                    event_type = ev.kind.type_num(),
                                    msg_id = id,
                                    source = context.source,
                                    operation_id = operation_id.as_deref().unwrap_or_default(),
                                    message_map_hit = true,
                                    reason = "chain_declaration_projected_by_chain_event",
                                    "type=2 接龙声明由 canonical chain projection 投影，跳过普通 message patch"
                                );
                            }
                            continue;
                        }
                        let op = crate::channel::edit_content_op(id, fields);
                        if let Some(context) = observation
                            .filter(|context| context.target_matches(Some(id), Some(fields)))
                        {
                            let meta = crate::sync::observability::storage_op_metadata(&op);
                            let input_meta =
                                crate::sync::observability::post_fields_metadata(fields);
                            tracing::info!(
                                hop = "sync.type2.patch",
                                corr = context.corr,
                                track_id = context.track_id.as_str(),
                                channel_id = ev.channel_id.as_str(),
                                event_seq = ev.seq.0,
                                event_type = ev.kind.type_num(),
                                msg_id = id,
                                source = context.source,
                                operation_id = operation_id.as_deref().unwrap_or_default(),
                                message_map_hit = true,
                                operation = %meta["operation"],
                                key_col = %meta["key_col"],
                                patch_columns = %meta["patch_columns"],
                                patch_field_meta = %meta["patch_field_meta"],
                                field_presence = %input_meta["field_presence"],
                                field_lengths = %input_meta["field_lengths"],
                                field_hashes = %input_meta["field_hashes"],
                                "type=2 消息更新输入已准备落库"
                            );
                        }
                        ops.push(op);
                    } else if let Some(context) =
                        observation.filter(|context| context.target_matches(Some(id), None))
                    {
                        tracing::warn!(
                            hop = "sync.event.skipped",
                            corr = context.corr,
                            track_id = context.track_id.as_str(),
                            channel_id = ev.channel_id.as_str(),
                            event_seq = ev.seq.0,
                            event_type = ev.kind.type_num(),
                            msg_id = id,
                            source = context.source,
                            operation_id = operation_id.as_deref().unwrap_or_default(),
                            message_map_hit = false,
                            reason = "message_map_miss",
                            "type=2 消息更新缺少 messages map，未生成补丁"
                        );
                    }
                }
            }
            EventKind::PostRevoke => {
                if let Some(id) = msg_id {
                    let op = crate::channel::revoke_op(id);
                    if let Some(context) =
                        observation.filter(|context| context.target_matches(Some(id), None))
                    {
                        let meta = crate::sync::observability::storage_op_metadata(&op);
                        tracing::info!(
                            hop = "sync.type3.revoke",
                            corr = context.corr,
                            track_id = context.track_id.as_str(),
                            channel_id = ev.channel_id.as_str(),
                            event_seq = ev.seq.0,
                            event_type = ev.kind.type_num(),
                            msg_id = id,
                            source = context.source,
                            operation_id = operation_id.as_deref().unwrap_or_default(),
                            message_map_hit = messages.contains_key(id),
                            operation = %meta["operation"],
                            key_col = %meta["key_col"],
                            patch_columns = %meta["patch_columns"],
                            "type=3 撤回操作已准备落库"
                        );
                    }
                    ops.push(op);
                }
            }
            EventKind::PostRead => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        if !fields.read_bits.is_empty() {
                            let op = crate::channel::apply_read_op(id, &fields.read_bits);
                            if let Some(context) = observation
                                .filter(|context| context.target_matches(Some(id), Some(fields)))
                            {
                                let meta = crate::sync::observability::storage_op_metadata(&op);
                                tracing::info!(
                                    hop = "sync.type6.read_bits",
                                    corr = context.corr,
                                    track_id = context.track_id.as_str(),
                                    channel_id = ev.channel_id.as_str(),
                                    event_seq = ev.seq.0,
                                    event_type = ev.kind.type_num(),
                                    msg_id = id,
                                    source = context.source,
                                    operation_id = operation_id.as_deref().unwrap_or_default(),
                                    message_map_hit = true,
                                    operation = %meta["operation"],
                                    key_col = %meta["key_col"],
                                    read_bits_present = true,
                                    read_bits_hash = %meta["patch_field_meta"]["read_bits"]["hash"],
                                    reader_id = ev.actor_id.as_str(),
                                    patch_columns = %meta["patch_columns"],
                                    patch_field_meta = %meta["patch_field_meta"],
                                    read_bits_length = fields.read_bits.len(),
                                    "type=6 已读位更新操作已准备落库"
                                );
                            }
                            ops.push(op);
                        } else if let Some(context) = observation
                            .filter(|context| context.target_matches(Some(id), Some(fields)))
                        {
                            tracing::info!(
                                hop = "sync.event.skipped",
                                corr = context.corr,
                                track_id = context.track_id.as_str(),
                                channel_id = ev.channel_id.as_str(),
                                event_seq = ev.seq.0,
                                event_type = ev.kind.type_num(),
                                msg_id = id,
                                source = context.source,
                                operation_id = operation_id.as_deref().unwrap_or_default(),
                                message_map_hit = true,
                                reason = "read_bits_empty",
                                "type=6 缺少 read_bits，未生成已读补丁"
                            );
                        }
                    } else if let Some(context) =
                        observation.filter(|context| context.target_matches(Some(id), None))
                    {
                        tracing::warn!(
                            hop = "sync.event.skipped",
                            corr = context.corr,
                            track_id = context.track_id.as_str(),
                            channel_id = ev.channel_id.as_str(),
                            event_seq = ev.seq.0,
                            event_type = ev.kind.type_num(),
                            msg_id = id,
                            source = context.source,
                            operation_id = operation_id.as_deref().unwrap_or_default(),
                            message_map_hit = false,
                            reason = "message_map_miss",
                            "type=6 已读位未命中 messages map，未生成补丁"
                        );
                    }
                }
            }
            EventKind::ChannelTerminalClosed => {}
            EventKind::Other(_) => {}
        }
    }
    ops
}

#[cfg(test)]
#[path = "sync_effects_tests.rs"]
mod tests;

pub fn sync_channel_update_plans(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<crate::channel_update::PendingChannelUpdate> {
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
        plans.push(crate::channel_update::PendingChannelUpdate::new(
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
