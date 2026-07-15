//! Dialog-row projection emit factories.

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

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
        "repliedMessage": serde_json::from_str::<serde_json::Value>(&fields.replied_message)
            .unwrap_or(serde_json::Value::Null),
        "replyMessages": serde_json::from_str::<serde_json::Value>(&fields.reply_messages)
            .unwrap_or_else(|_| serde_json::json!([])),
        "replyCount": fields.reply_count,
        "userSnapshot": user_snapshot,
        "createAt": fields.create_at,
        "eventSeq": event_seq,
    })
}
