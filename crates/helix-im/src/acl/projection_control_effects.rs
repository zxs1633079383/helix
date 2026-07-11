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
            },
            "readBits": "",
            "text": text,
            "type": msg_type,
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

#[cfg(test)]
#[path = "projection_control_effects_tests.rs"]
mod tests;

/// C3：type=1 可见新消息 → `im:channel:update-by-post`。
pub fn emit_channel_update_by_post(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    update: &crate::channel_write::PostChannelUpdate,
    projection: &crate::dialog_patch::DialogPatchProjection,
    source: &str,
) -> Effect {
    use serde_json::json;
    let last_post = post_fields_json(channel_id, event_seq, msg_id, fields);
    let message_class = crate::channel_write::message_class(update);
    let mention_delta = if update.mention_hit { 1 } else { 0 };
    let urgent_delta = if update.urgent_hit { 1 } else { 0 };
    let dialog_patch = json!({
        "id": channel_id.as_str(),
        "channelId": channel_id.as_str(),
        "lastPost": last_post.clone(),
        "lastMessage": update.last_message.clone(),
        "unread": projection.unread_count,
        "unreadCount": projection.unread_count,
        "mention": update.mention_hit,
        "mentionCount": projection.mention_count,
        "urgent": update.urgent_hit,
        "urgentCount": projection.urgent_count,
        "unreadPostId": projection.unread_post_id.clone(),
        "lastPostAt": update.msg_create_at,
        "lastRootPostAt": projection.last_root_post_at,
        "eventSeq": event_seq,
        "lastEventSeq": event_seq,
        "mentionedUserIds": update.mentions.clone(),
        "urgentUserIds": update.urgent_user_ids.clone(),
        "mentionList": projection.mention_list.clone(),
        "urgentPostList": projection.urgent_post_list.clone(),
    });
    let payload = json!({
        "event": "im:channel:update-by-post",
        "data": {
            "op": "dialog.patchByPost",
            "source": source,
            "channel_id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "event_seq": event_seq,
            "eventSeq": event_seq,
            "msg_id": msg_id,
            "msgId": msg_id,
            "messageClass": message_class,
            "visible": update.visible,
            "senderUserId": update.sender_user_id.clone(),
            "lastPost": last_post.clone(),
            "lastMessage": update.last_message.clone(),
            "unreadDelta": update.unread_delta,
            "mentionDelta": mention_delta,
                "urgentDelta": urgent_delta,
                "mentionedUserIds": update.mentions.clone(),
                "urgentUserIds": update.urgent_user_ids.clone(),
                "mentionList": projection.mention_list.clone(),
                "urgentPostList": projection.urgent_post_list.clone(),
                "dialogPatch": dialog_patch,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update_by_post: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 后端 `update_channel` 定向成员绝对态 → 左侧 Dialog 行 badge/lastPost 指令。
pub fn emit_channel_update_by_member_dialog(
    channel_id: ChannelId,
    projection: &crate::dialog_patch::MemberDialogProjection,
    source: &str,
) -> Effect {
    use serde_json::json;
    let last_post = projection
        .last_post
        .clone()
        .unwrap_or(serde_json::Value::Null);
    let sender_user_id = last_post
        .get("userId")
        .or_else(|| last_post.get("user_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_post_id = last_post
        .get("id")
        .or_else(|| last_post.get("postId"))
        .and_then(|v| v.as_str());
    let urgent_hit = last_post_id
        .and_then(|id| {
            projection
                .urgent_post_list
                .as_ref()
                .map(|items| items.iter().any(|item| item == id))
        })
        .unwrap_or(false);
    let mention_hit = last_post_id
        .and_then(|id| {
            projection
                .mention_list
                .as_ref()
                .map(|items| items.iter().any(|item| item == id))
        })
        .unwrap_or(false)
        || last_post
            .get("mentions")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(&projection.user_id))
            })
            .unwrap_or(false);
    let message_class = match (mention_hit, urgent_hit) {
        (true, true) => "mention_urgent",
        (true, false) => "mention",
        (false, true) => "urgent",
        (false, false) => "normal",
    };
    let last_message = last_post
        .get("message")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let dialog_patch = json!({
        "id": channel_id.as_str(),
        "channelId": channel_id.as_str(),
        "userId": projection.user_id.clone(),
        "projectionAuthority": "server-member-absolute",
        "memberScope": "current_user",
        "lastPost": last_post,
        "lastMessage": last_message.clone(),
        "unreadCount": projection.unread_count,
        "unread": projection.unread_count,
        "unreadPostId": projection.unread_post_id.clone(),
        "mentionCount": projection.mention_count,
        "mentionCountRoot": projection.mention_count_root,
        "mentionList": projection.mention_list.clone(),
        "mentionUser": projection.mention_user.clone(),
        "urgentCount": projection.urgent_count,
        "urgentPostList": projection.urgent_post_list.clone(),
        "urgentMentionUser": projection.urgent_mention_user.clone(),
        "msgCount": projection.msg_count,
        "msgCountRoot": projection.msg_count_root,
        "msgCountPrivate": projection.msg_count_private,
        "lastReadSeq": projection.last_read_seq,
        "lastPostAt": projection.last_post_at,
        "lastRootPostAt": projection.last_root_post_at,
    });
    let data = json!({
        "op": "dialog.patchByMember",
        "source": source,
        "projectionAuthority": "server-member-absolute",
        "memberScope": "current_user",
        "channel_id": channel_id.as_str(),
        "channelId": channel_id.as_str(),
        "userId": projection.user_id.clone(),
        "messageClass": message_class,
        "senderUserId": sender_user_id,
        "lastPost": projection.last_post.clone().unwrap_or(serde_json::Value::Null),
        "lastMessage": last_message,
        "unreadCount": projection.unread_count,
        "mentionCount": projection.mention_count,
        "urgentCount": projection.urgent_count,
        "mentionList": projection.mention_list.clone(),
        "urgentPostList": projection.urgent_post_list.clone(),
        "msgCount": projection.msg_count,
        "lastReadSeq": projection.last_read_seq,
        "dialogPatch": dialog_patch,
    });
    let payload = json!({
        "event": "im:channel:update-by-post",
        "data": data,
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update_by_member_dialog: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

fn post_fields_json(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> serde_json::Value {
    let props = serde_json::from_str::<serde_json::Value>(&fields.props)
        .unwrap_or_else(|_| serde_json::json!({}));
    let user_snapshot = serde_json::from_str::<serde_json::Value>(&fields.user_snapshot)
        .unwrap_or_else(|_| serde_json::json!({}));
    let expedite_map = serde_json::from_str::<serde_json::Value>(&fields.expedite_map)
        .unwrap_or_else(|_| serde_json::json!({}));
    let topic = serde_json::from_str::<serde_json::Value>(&fields.topic)
        .unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "id": if fields.id.is_empty() { msg_id } else { fields.id.as_str() },
        "temporaryId": fields.temporary_id.clone(),
        "channelId": if fields.channel_id.is_empty() {
            channel_id.as_str()
        } else {
            fields.channel_id.as_str()
        },
        "userId": fields.user_id.clone(),
        "type": fields.msg_type.clone(),
        "message": fields.message.clone(),
        "simpleMessage": fields.simple_message.clone(),
        "props": props,
        "viewers": fields.viewers.clone(),
        "mentions": fields.mentions.clone(),
        "expediteMap": expedite_map,
        "topic": topic,
        "replyId": fields.reply_id.clone(),
        "replyRootId": fields.reply_root_id.clone(),
        "replyFirstLevelId": fields.reply_first_level_id.clone(),
        "userSnapshot": user_snapshot,
        "createAt": fields.create_at,
        "eventSeq": event_seq,
    })
}
