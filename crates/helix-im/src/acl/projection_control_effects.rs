//! Thin, pass-through, and control projection emit factories.

use crate::state::{ChannelId, Seq};
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

/// 发送乐观上屏事件（im:post:sending）。
pub fn emit_post_sending(
    channel_id: ChannelId,
    temporary_id: &str,
    text: &str,
    msg_type: &str,
) -> Effect {
    emit_post_sending_with_reply(channel_id, temporary_id, text, msg_type, "", "", "")
}

pub fn emit_post_sending_with_reply(
    channel_id: ChannelId,
    temporary_id: &str,
    text: &str,
    msg_type: &str,
    reply_id: &str,
    reply_root_id: &str,
    reply_first_level_id: &str,
) -> Effect {
    use serde_json::json;
    // type 空串容错 "TEXT"（render-ready·前端不再 pendingType 兜底）。
    let msg_type = if msg_type.is_empty() {
        "TEXT"
    } else {
        msg_type
    };
    let payload = json!({
        "event": "im:post:sending",
        "data": {
            // 信号锚（snake·历史兼容）。
            "channel_id": channel_id.as_str(),
            "temporary_id": temporary_id,
            // —— render-ready 乐观终态行（S6·issue #55·壳不造乐观·壳直绑·不再取 pendingText/pendingType）——
            "msgId": temporary_id,
            "temporaryId": temporary_id,
            "channelId": channel_id.as_str(),
            "eventSeq": serde_json::Value::Null,
            "sendStatus": "sending",
            "serverId": serde_json::Value::Null,
            "isSelf": true,
            "actionCapabilities": {
                "canReply": false,
                "canForward": false,
                "canReact": false,
                "canUrgent": false,
                "canCreateTopic": false,
                "canRevoke": false,
                "canRetry": false,
                "canUrgentConfirm": false,
                "canMarkRead": false,
                "canViewReceipt": false,
                "canTemplateConfirm": false,
                "canLocate": false,
            },
            "readBits": "",
            "text": text,
            "type": msg_type,
            "replyId": reply_id,
            "replyRootId": reply_root_id,
            "replyFirstLevelId": reply_first_level_id,
            "replyCount": 0,
            "replyMessages": [],
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_sending: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 发送失败渲染补丁（im:post:send-failed）。
///
/// HTTP PortError 真实失败路径触发，按 temporaryId 锚定已有乐观行；DB 终态仍是
/// `send_status='unsend'`，UI 渲染值为 `sendStatus='failed'`。
pub fn emit_post_send_failed(channel_id: &str, temporary_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:send-failed",
        "data": {
            "channelId": channel_id,
            "temporaryId": temporary_id,
            "sendStatus": "failed",
            "serverId": serde_json::Value::Null,
            "isSelf": true,
            "actionCapabilities": {
                "canReply": false,
                "canForward": false,
                "canReact": false,
                "canUrgent": false,
                "canCreateTopic": false,
                "canRevoke": false,
                "canRetry": true,
                "canUrgentConfirm": false,
                "canMarkRead": false,
                "canViewReceipt": false,
                "canTemplateConfirm": false,
                "canLocate": false,
            },
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_send_failed: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 发送 im:sync:too_long 事件。
pub fn emit_sync_too_long(channel_id: ChannelId, reset_to: Seq) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:sync:too_long",
        "data": {
            "channelId": channel_id.as_str(),
            "resetTo": reset_to.0,
            "mode": "drop_and_reload",
            "dropLocalMessages": true,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_too_long: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// G-S3：同步事务成功提交后的 render-ready 状态。HTTP 成功或解析成功均不得直接调用。
pub fn emit_sync_state(channel_id: ChannelId, committed_seq: Seq) -> Effect {
    emit_sync_state_with_trigger(
        channel_id,
        committed_seq,
        crate::state::SyncTrigger::Routine,
    )
}

pub fn emit_sync_state_with_trigger(
    channel_id: ChannelId,
    committed_seq: Seq,
    trigger: crate::state::SyncTrigger,
) -> Effect {
    use serde_json::json;
    let mut payload = json!({
        "event": "im:sync:state",
        "data": {
            "state": "committed",
            "recovery": {
                "channelId": channel_id.as_str(),
                "committedSeq": committed_seq.0,
            }
        }
    });
    if let (Some(label), Some(data)) = (
        trigger.projection_label(),
        payload
            .get_mut("data")
            .and_then(serde_json::Value::as_object_mut),
    ) {
        data.insert("trigger".to_string(), label.into());
    }
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_state: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// A4：发送 im:connection:established 事件（hello 握手完成信号）。
pub fn emit_connection_established(connection_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:connection:established",
        "data": {
            "connectionId": connection_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_connection_established: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channels:loaded 事件（hello 冷启动补齐信号）。
pub fn emit_channels_loaded() -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channels:loaded",
        "data": { "items": [] }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channels_loaded: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channel:increment 事件（透传 increment_channel 帧 data）。
pub fn emit_channel_increment(channel_id: ChannelId, raw: &[u8]) -> Effect {
    use serde_json::json;
    let inc_data: serde_json::Value =
        serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    let payload = json!({
        "event": "im:channel:increment",
        "data": {
            "channel_id": channel_id.as_str(),
            "increment": inc_data,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_increment: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channel:update 事件（子 topic 批次结束）。
pub fn emit_channel_update(channel_id: ChannelId) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:update",
        "data": { "channel_id": channel_id.as_str() }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 连接控制：`im_reconnect` → `im:net:reconnect_requested` 信号。
pub fn emit_reconnect_requested() -> Effect {
    let payload = serde_json::json!({ "event": "im:net:reconnect_requested", "data": {} });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_reconnect_requested: static JSON must not fail"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// `posts_update`（批量编辑/撤回）→ `im:post:batch-updated`。
pub fn emit_post_batch_updated(channel_id: ChannelId, posts: &serde_json::Value) -> Effect {
    use serde_json::json;
    let posts = if posts.is_array() {
        crate::render_ready::shape_message_rows(posts)
    } else {
        json!([])
    };
    let payload = json!({
        "event": "im:post:batch-updated",
        "data": {
            "channel_id": channel_id.as_str(),
            "posts": posts,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_batch_updated: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

mod dialog;
pub use dialog::{member_dialog_patch_data, post_dialog_patch_data};

#[cfg(test)]
#[path = "projection_control_effects_tests.rs"]
mod tests;
