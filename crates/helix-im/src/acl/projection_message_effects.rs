//! Message-row projection emit factories.

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

/// spec06 FixB：message-row 信号的完整可渲染 payload 构造（数据在手，零回读）。
fn message_item_data(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> serde_json::Value {
    use serde_json::json;
    let props: serde_json::Value = if fields.props.is_empty() {
        json!({})
    } else {
        serde_json::from_str(&fields.props).unwrap_or_else(|_| json!({}))
    };
    json!({
        "channel_id": channel_id.as_str(),
        "event_seq": event_seq,
        "msg_id": msg_id,
        "temporaryId": fields.temporary_id.as_str(),
        "channelId": channel_id.as_str(),
        "userId": fields.user_id.as_str(),
        "type": fields.msg_type.as_str(),
        "message": fields.message.as_str(),
        "props": props,
        "createAt": fields.create_at,
        "updateAt": fields.create_at,
        "readBits": fields.read_bits.as_str(),
        "viewers": fields.viewers,
    })
}

/// 序列化 ImDomainEvent 为 DomainEventBytes（Emit 的载荷）。
pub fn emit_post_received(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:received",
        "data": message_item_data(channel_id, event_seq, msg_id, fields),
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
    use serde_json::json;
    let payload = json!({
        "event": "im:post:read",
        "data": message_item_data(channel_id, event_seq, msg_id, fields),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_read: static JSON shape must not fail to serialize"),
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
        "data": message_item_data(channel_id, event_seq, msg_id, fields),
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
    use serde_json::json;
    let payload = json!({
        "event": "im:post:updated",
        "data": message_item_data(channel_id, event_seq, msg_id, fields),
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
    use serde_json::json;
    let payload = json!({
        "event": "im:post:deleted",
        "data": message_item_data(channel_id, event_seq, msg_id, fields),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_deleted: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}
