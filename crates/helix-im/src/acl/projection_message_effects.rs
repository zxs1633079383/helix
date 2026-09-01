//! Message-row projection emit factories.

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

mod data;
use data::message_item_data;

pub fn emit_post_received(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_post_received_for_viewer(channel_id, event_seq, msg_id, fields, "")
}

pub fn emit_post_received_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    emit_post_received_for_viewer_with_path(
        channel_id,
        event_seq,
        msg_id,
        fields,
        viewer_user_id,
        "live_ws",
    )
}

/// 为 canonical durable stream 发布统一的 `im:post:received` 终态，同时保留 viewer-local 字段。
///
/// Go 的 canonical audience 已包含发送者；发送者也必须沿用 `received` 合同，不能把 canonical
/// 事件误分类为 `sent`。`isSelf` 仍由 `viewer_user_id` 计算，故不牺牲本端消息样式。
pub fn emit_post_received_for_canonical_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    emit_post_received_for_viewer_with_path_and_event(
        channel_id,
        event_seq,
        msg_id,
        fields,
        viewer_user_id,
        "live_ws",
        "im:post:received",
    )
}

/// 按可信入站路径发布消息终态；来源只使用固定枚举，不携带用户或频道标识。
pub fn emit_post_received_for_viewer_with_path(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
    telemetry_path: &'static str,
) -> Effect {
    let event = if !viewer_user_id.is_empty() && fields.user_id == viewer_user_id {
        "im:post:sent"
    } else {
        "im:post:received"
    };
    emit_post_received_for_viewer_with_path_and_event(
        channel_id,
        event_seq,
        msg_id,
        fields,
        viewer_user_id,
        telemetry_path,
        event,
    )
}

/// 组装消息终态字节；调用方决定事件名，避免 canonical 与 sync 的 viewer 语义互相污染。
fn emit_post_received_for_viewer_with_path_and_event(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
    telemetry_path: &'static str,
    event: &'static str,
) -> Effect {
    use serde_json::json;
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false);
    if let Some(object) = data.as_object_mut() {
        object.insert("telemetryPath".to_string(), json!(telemetry_path));
    }
    let payload = json!({
        "event": event,
        "data": data,
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_received: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 在线 `post_read`（type=6）→ `im:post:read`。
pub fn emit_post_read(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_post_read_for_viewer(channel_id, event_seq, msg_id, fields, "")
}

pub fn emit_post_read_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    emit_post_read_with_receipt_revision_for_viewer(
        channel_id,
        event_seq,
        msg_id,
        fields,
        0,
        viewer_user_id,
    )
}

/// 在线 `post_read` 的 render-ready 回执失效版本。
///
/// Angular 只比较这个不透明版本决定是否重取人员回执，不接触或解释 `readBits`。
pub fn emit_post_read_with_receipt_revision(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    receipt_revision: i64,
) -> Effect {
    emit_post_read_with_receipt_revision_for_viewer(
        channel_id,
        event_seq,
        msg_id,
        fields,
        receipt_revision,
        "",
    )
}

pub fn emit_post_read_with_receipt_revision_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false);
    data["receiptRevision"] = json!(receipt_revision);
    let payload = json!({
        "event": "im:post:read",
        "data": data,
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_read: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 离线 sync type6 的 sender 视角回执。
///
/// `reader_id` 只能来自持久化 ChannelEvent.actorId；调用方还必须确认同一 msgId 的
/// `messages` 快照携权威 readBits。这里不解析位图反推用户，避免成员快照漂移时张冠李戴。
pub fn emit_sync_post_read(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    reader_id: &str,
    receipt_revision: i64,
) -> Effect {
    emit_sync_post_read_for_viewer(
        channel_id,
        event_seq,
        msg_id,
        fields,
        reader_id,
        receipt_revision,
        "",
    )
}

pub fn emit_sync_post_read_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    reader_id: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false);
    data["postId"] = json!(msg_id);
    data["readerId"] = json!(reader_id);
    data["receiptRevision"] = json!(receipt_revision);
    let payload = json!({
        "event": "im:post:read",
        "data": data,
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_post_read: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 离线 sync 应用的 type=6 read → `im:channel:read_echo`。
pub fn emit_channel_read_echo(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_channel_read_echo_for_viewer(channel_id, event_seq, msg_id, fields, "")
}

pub fn emit_channel_read_echo_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:read_echo",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_read_echo: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=2 编辑 → `im:post:updated`（仅可见时 emit）。
pub fn emit_post_updated(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_post_updated_for_viewer(channel_id, event_seq, msg_id, fields, "")
}

pub fn emit_post_updated_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:updated",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_updated: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=3 撤回 → `im:post:deleted`。
pub fn emit_post_deleted(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_post_deleted_for_viewer(channel_id, event_seq, msg_id, fields, "", "", 0, "")
}

/// 撤回 viewer-ready 系统事件。旧 `im:post:deleted` 事件名与 fat MessageItemData 保持兼容，
/// 系统事件字段 additive 增加；actor 缺失时明确 unavailable，绝不从原消息 `userId` 推断。
#[allow(clippy::too_many_arguments)]
pub fn emit_post_deleted_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    event_id: &str,
    actor_id: &str,
    occurred_at: i64,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let actor_available = !actor_id.is_empty();
    let is_self = actor_available && actor_id == viewer_user_id;
    let display_text = if is_self {
        "你撤回了一条消息"
    } else {
        "某人撤回了一条消息"
    };
    let stable_event_id = if event_id.starts_with("revoke:") {
        event_id.to_string()
    } else {
        // 离线 ChannelEvent.id 是数据库行 ID；在线 Go 合同以 channel+seq 派生跨面 eventId。
        format!("revoke:{}:{}", channel_id.as_str(), event_seq)
    };
    // viewer 置空，阻断 message_item_data 按原消息作者计算 isSelf；随后只按 actor 重写。
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, "", true);
    let object = data
        .as_object_mut()
        .expect("recall projection starts from a static JSON object");
    object.insert("recalledText".to_string(), json!(fields.message));
    object.insert("type".to_string(), json!("system"));
    object.insert("message".to_string(), json!(display_text));
    object.insert("text".to_string(), json!(display_text));
    object.insert("systemNotice".to_string(), json!(true));
    object.insert("isSelf".to_string(), json!(is_self));
    object.insert("kind".to_string(), json!("system"));
    object.insert("system".to_string(), json!(true));
    object.insert("systemEvent".to_string(), json!("message-recalled"));
    object.insert("eventId".to_string(), json!(stable_event_id));
    object.insert("actorId".to_string(), json!(actor_id));
    object.insert("actorAvailable".to_string(), json!(actor_available));
    object.insert("subjectMemberIds".to_string(), json!([]));
    object.insert("occurredAt".to_string(), json!(occurred_at.max(0)));
    object.insert("displayText".to_string(), json!(display_text));
    object.insert("previewText".to_string(), json!(display_text));
    object.insert("recalledMsgId".to_string(), json!(msg_id));
    object.insert("targetMsgId".to_string(), json!(msg_id));
    let payload = json!({
        "event": "im:post:deleted",
        "data": data,
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_deleted: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// canonical durable stream 的在线撤回事实，保持 `posts_update` 的 `im:post:revoke` 合同。
///
/// 离线 sync 仍由 `emit_post_deleted_for_viewer` 负责系统占位；canonical 在线事件只携
/// authority 的最小 identity/字段，避免把离线展示语义泄漏到实时撤回协议。
pub fn emit_post_revoke_for_canonical_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;

    let mut data = json!({
        "id": msg_id,
        "channelId": channel_id.as_str(),
        "eventSeq": event_seq,
        "revoke": true,
    });
    let object = data
        .as_object_mut()
        .expect("canonical revoke projection starts from a static JSON object");
    for (key, value) in [
        ("temporaryId", fields.temporary_id.as_str()),
        ("userId", fields.user_id.as_str()),
        ("type", fields.msg_type.as_str()),
        ("message", fields.message.as_str()),
    ] {
        if !value.is_empty() {
            object.insert(key.to_string(), json!(value));
        }
    }
    if fields.create_at > 0 {
        object.insert("createAt".to_string(), json!(fields.create_at));
    }
    if fields.update_at > 0 {
        object.insert("updateAt".to_string(), json!(fields.update_at));
    }
    crate::event::post::revoke(data)
        .expect("emit_post_revoke_for_canonical_viewer: static JSON shape must serialize")
        .into_effect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// canonical 自端终态固定为 received，但 payload 仍保留 isSelf viewer 视角。
    #[test]
    fn canonical_sender_projection_is_received_with_self_marker() {
        let channel_id = crate::state::test_channel_id(41);
        let fields = crate::sync_session::PostFields {
            id: "post-41".to_string(),
            user_id: "viewer-41".to_string(),
            temporary_id: "tmp-41".to_string(),
            message: "hello".to_string(),
            ..Default::default()
        };

        let Effect::Emit { event } =
            emit_post_received_for_canonical_viewer(channel_id, 7, "post-41", &fields, "viewer-41")
        else {
            panic!("canonical projection must emit a domain event");
        };
        let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();

        assert_eq!(payload["event"], "im:post:received");
        assert_eq!(payload["data"]["isSelf"], true);
        assert_eq!(payload["data"]["temporaryId"], "tmp-41");
    }

    /// sync/replay 的 sender 视角仍保留 sent，避免改变既有 viewer-local 合同。
    #[test]
    fn sync_sender_projection_remains_sent() {
        let channel_id = crate::state::test_channel_id(42);
        let fields = crate::sync_session::PostFields {
            user_id: "viewer-42".to_string(),
            ..Default::default()
        };

        let Effect::Emit { event } = emit_post_received_for_viewer_with_path(
            channel_id,
            8,
            "post-42",
            &fields,
            "viewer-42",
            "sync_replay",
        ) else {
            panic!("sync projection must emit a domain event");
        };
        let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();

        assert_eq!(payload["event"], "im:post:sent");
        assert_eq!(payload["data"]["telemetryPath"], "sync_replay");
    }

    /// canonical 在线撤回沿用 posts_update 的最小 revoke 事实，不误发离线 deleted 占位。
    #[test]
    fn canonical_revoke_projection_uses_online_revoke_event() {
        let channel_id = crate::state::test_channel_id(43);
        let fields = crate::sync_session::PostFields {
            id: "post-43".to_string(),
            message: "hello".to_string(),
            ..Default::default()
        };

        let Effect::Emit { event } =
            emit_post_revoke_for_canonical_viewer(channel_id, 9, "post-43", &fields)
        else {
            panic!("canonical revoke projection must emit a domain event");
        };
        let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();

        assert_eq!(payload["event"], "im:post:revoke");
        assert_eq!(payload["data"]["id"], "post-43");
        assert_eq!(payload["data"]["channelId"], channel_id.as_str());
        assert_eq!(payload["data"]["eventSeq"], 9);
        assert_eq!(payload["data"]["revoke"], true);
        assert_eq!(payload["data"]["message"], "hello");
        assert!(payload["data"].get("systemEvent").is_none());
    }
}
