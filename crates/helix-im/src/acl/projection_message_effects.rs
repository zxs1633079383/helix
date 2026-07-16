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
    use serde_json::json;
    let payload = json!({
        "event": "im:post:received",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false),
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
    emit_post_read_with_receipt_revision(channel_id, event_seq, msg_id, fields, 0)
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
    use serde_json::json;
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, "", false);
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
    use serde_json::json;
    let mut data = message_item_data(channel_id, event_seq, msg_id, fields, "", false);
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
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:read_echo",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, "", false),
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
