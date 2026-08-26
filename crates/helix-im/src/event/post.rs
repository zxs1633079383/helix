//! Post 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

mod forward;
mod reaction;
mod template;
mod urgent;

/// 保存一次 authority 投影生成的 post 终态与 channel lastPost。
pub struct AuthorityProjection {
    pub received_data: Value,
    pub last_post: Value,
}

/// 构造本地发送中事实。
pub fn sending(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:sending", data)
}

/// 从本地待发送 body 构造只含业务字段的 optimistic 事实。
pub fn sending_from_local_body(
    channel_id: &str,
    temporary_id: &str,
    user_id: &str,
    body: &Value,
) -> Result<MessageV3Event, crate::ImError> {
    let mut data = serde_json::json!({
        "channelId": channel_id,
        "temporaryId": temporary_id,
        "message": body.get("message").and_then(Value::as_str).unwrap_or(""),
        "type": body.get("type").and_then(Value::as_str).unwrap_or("TEXT"),
        "createAt": body.get("createAt").and_then(Value::as_i64).unwrap_or_default(),
        "sendStatus": "sending",
    });
    if !user_id.is_empty() {
        data["userId"] = serde_json::json!(user_id);
    }
    for key in [
        "simpleMessage",
        "props",
        "viewers",
        "mentions",
        "repliedMessage",
        "userSnapshot",
    ] {
        if let Some(value) = body.get(key) {
            data[key] = value.clone();
        }
    }
    for key in ["replyId", "replyRootId", "replyFirstLevelId"] {
        if let Some(value) = body
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            data[key] = serde_json::json!(value);
        }
    }
    sending(data)
}

/// 构造服务端已接收消息事实。
pub fn received(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:received", data)
}

/// 构造客户端 ACK 的真实 HTTP 终态，供 Host 在 Emit 边界沉淀成功率。
pub fn client_ack_terminal(
    platform: crate::module::ClientPlatform,
    succeeded: bool,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        if succeeded {
            "im:post:client-ack-succeeded"
        } else {
            "im:post:client-ack-failed"
        },
        serde_json::json!({
            "platform": platform.as_str(),
        }),
    )
}

/// 从 durable message row 生成 G-14a 模板确认绝对态。
pub fn template_update_from_row(
    row: &helix_core::effect::Row,
) -> Result<Option<MessageV3Event>, crate::ImError> {
    template::from_row(row)
}

/// 判断 props 是否携带服务端模板确认集合。
pub fn has_template_confirmation(props: &str) -> bool {
    template::has_confirmation(props)
}

/// `im:post:received` 参考包声明的**可选文本字段**投影表。
///
/// 键集权威 = `specs/003-messagev3-gate-inventory/gates/post/mv3-g01{a,b,f,h,k}/outbound-sample.json`
/// 中 `im:post:received` 的 `data` 键并集；每一项在这里都必须能指到 `PostFields` 上的权威来源。
///
/// **不臆造默认值**：权威没给（空串）就不投影这个键——参考包只声明「有值时长什么样」，
/// 没有声明缺省值，凭空补 `""` 会让前端把「服务端没说」误读成「服务端说了空」。
///
/// 例：回复三锚在此之前只落库不投影，`sending` 阶段带三锚的乐观引用条会被 echo 回来的
/// `received` 覆盖掉——引用关系在对账瞬间丢失（MV3-G01k 序 2 sender/receiver 两侧均已声明）。
const RECEIVED_DECLARED_TEXT_FIELDS: [(&str, fn(&crate::sync_session::PostFields) -> &str); 4] = [
    ("simpleMessage", |fields| fields.simple_message.as_str()),
    ("replyId", |fields| fields.reply_id.as_str()),
    ("replyRootId", |fields| fields.reply_root_id.as_str()),
    ("replyFirstLevelId", |fields| {
        fields.reply_first_level_id.as_str()
    }),
];

/// 参考包已声明、但当前 `PostFields` 没有权威来源、因此**无法**投影的 `im:post:received` 键。
///
/// - `receipt`：`{recipientCount, readCount, unreadCount, allRead}` 由 `post_receipt` /
///   `receiver_snapshot` 聚合得出，不在单条 post authority 里。
///
/// 两者都需要先扩 parser + `PostFields`（本文件之外），在那之前**宁缺勿造**——
/// 造一个 `receipt: {readCount: 0}` 会让 UI 把「未知」渲染成「确定没人读」。
/// 漂移登记见 `tests/message_v3/post/mv3_g01h/main.rs::RECEIVED_DECLARED_DRIFT`。
pub const RECEIVED_UNPROJECTABLE_DECLARED_FIELDS: [&str; 1] = ["receipt"];

/// 从 authority 一次解析 props，并生成 post 与 channel 共用的真实结构。
pub fn authority_projection(
    event: &crate::sync_session::EventEnvelope,
) -> Result<AuthorityProjection, crate::ImError> {
    let fields = &event.fields;
    let mut received_data = serde_json::json!({
        "id": fields.id,
        "channelId": event.channel_id.as_str(),
        "temporaryId": fields.temporary_id,
        "eventSeq": event.seq.0,
        "userId": fields.user_id,
        "message": fields.message,
        "type": fields.msg_type,
        "createAt": fields.create_at,
        "sendStatus": "sent",
    });
    if !fields.user_snapshot.trim().is_empty() {
        received_data["userSnapshot"] = serde_json::from_str(&fields.user_snapshot)
            .map_err(|error| crate::ImError::Parse(format!("post user_snapshot: {error}")))?;
    }
    for (key, source) in RECEIVED_DECLARED_TEXT_FIELDS {
        let value = source(fields);
        if !value.is_empty() {
            received_data[key] = serde_json::json!(value);
        }
    }
    if !fields.snapshot_id.is_empty() {
        received_data["snapshotId"] = serde_json::json!(fields.snapshot_id);
    }
    if !fields.viewers.is_empty() {
        received_data["viewers"] = serde_json::json!(fields.viewers);
    }
    if !fields.replied_message.is_empty() {
        received_data["repliedMessage"] = serde_json::from_str(&fields.replied_message)
            .map_err(|error| crate::ImError::Parse(format!("post replied_message: {error}")))?;
    }
    let mut last_post = serde_json::json!({
        "id": fields.id,
        "userId": fields.user_id,
        "message": fields.message,
        "createAt": fields.create_at,
    });
    if let Some(snapshot) = received_data.get("userSnapshot").cloned() {
        last_post["userSnapshot"] = snapshot;
    }
    if event.fields.msg_type != "TEXT" {
        last_post["type"] = serde_json::json!(event.fields.msg_type.as_str());
    }
    let mut props = if event.fields.props.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str::<Value>(&event.fields.props)
            .map_err(|error| crate::ImError::Parse(format!("post props: {error}")))?
    };
    canonicalize_media_file_names(&mut props);
    if let Some(object) = props.as_object_mut() {
        object.remove("channel_event_seq");
    }
    if event.fields.msg_type.eq_ignore_ascii_case("MULTIPLY") {
        attach_shared_field(
            &mut received_data,
            &mut last_post,
            "forwardDetail",
            forward::detail(&props),
        );
    } else if !props.as_object().is_some_and(serde_json::Map::is_empty) && !props.is_null() {
        attach_shared_field(&mut received_data, &mut last_post, "props", props);
    }
    Ok(AuthorityProjection {
        received_data,
        last_post,
    })
}

/// 把 Go 回声中的媒体 `fileName` 兼容键收敛为 MessageV3 唯一 `name` 字段。
fn canonicalize_media_file_names(props: &mut Value) {
    let Some(object) = props.as_object_mut() else {
        return;
    };
    if let Some(file) = object.get_mut("file") {
        canonicalize_media_file_name(file);
    }
    if let Some(files) = object.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            canonicalize_media_file_name(file);
        }
    }
    if let Some(file) = object
        .get_mut("template")
        .and_then(Value::as_object_mut)
        .and_then(|template| template.get_mut("file"))
    {
        canonicalize_media_file_name(file);
    }
}

/// 单个媒体节点优先保留非空 canonical `name`，否则消费兼容 `fileName`。
fn canonicalize_media_file_name(file: &mut Value) {
    let Some(object) = file.as_object_mut() else {
        return;
    };
    let canonical_missing = object
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    let legacy = object
        .remove("fileName")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.is_empty());
    if canonical_missing {
        if let Some(name) = legacy {
            object.insert("name".to_string(), Value::String(name));
        }
    }
}

/// 把同一业务值写入 post 与 lastPost，边界处只做一次必要复制。
fn attach_shared_field(received_data: &mut Value, last_post: &mut Value, key: &str, value: Value) {
    received_data[key] = value.clone();
    last_post[key] = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChannelId, Seq};
    use crate::sync_session::{EventEnvelope, EventKind, PostFields};

    /// received 投影只携 Go 给出的 snapshotId，receipt 继续由独立查询权威提供。
    #[test]
    fn authority_projection_carries_snapshot_without_fabricating_receipt() {
        let channel_id =
            ChannelId::from_str("a9h5hrdsy3873dmg375a6ntqiw").expect("valid test channel");
        let event = EventEnvelope::new(
            channel_id,
            Seq(7),
            EventKind::PostUpsert,
            PostFields {
                id: "post-1".to_string(),
                snapshot_id: "snapshot-1".to_string(),
                user_id: "444".to_string(),
                user_snapshot: r#"{"userId":"444","userName":"破坏者"}"#.to_string(),
                ..PostFields::default()
            },
        );

        let projection = authority_projection(&event).expect("project authority");

        assert_eq!(projection.received_data["snapshotId"], "snapshot-1");
        assert_eq!(projection.received_data["userId"], "444");
        assert_eq!(
            projection.received_data["userSnapshot"]["userName"],
            "破坏者"
        );
        assert_eq!(projection.last_post["userSnapshot"]["userId"], "444");
        assert!(projection.received_data.get("receipt").is_none());
    }

    /// sending 投影必须沿用本地发送 body 的权威用户快照，避免首帧显示为空用户。
    #[test]
    fn sending_projection_carries_authoritative_user_snapshot() {
        let event = sending_from_local_body(
            "a9h5hrdsy3873dmg375a6ntqiw",
            "tmp-1",
            "444",
            &serde_json::json!({
                "message": "1",
                "type": "TEXT",
                "createAt": 7,
                "userSnapshot": {"userId":"444","userName":"破坏者"}
            }),
        )
        .expect("sending projection");
        let envelope: serde_json::Value =
            serde_json::from_slice(&event.into_bytes()).expect("event json");

        assert_eq!(envelope["data"]["userId"], "444");
        assert_eq!(envelope["data"]["userSnapshot"]["userName"], "破坏者");
    }

    /// Go 媒体回声的 `fileName` 别名必须在 Core 投影边界收敛，不能下放给平台 UI。
    #[test]
    fn authority_projection_canonicalizes_media_file_name() {
        let channel_id =
            ChannelId::from_str("a9h5hrdsy3873dmg375a6ntqiw").expect("valid test channel");
        let event = EventEnvelope::new(
            channel_id,
            Seq(8),
            EventKind::PostUpsert,
            PostFields {
                id: "post-audio-1".to_string(),
                msg_type: "AUDIO".to_string(),
                props: serde_json::json!({
                    "file": {
                        "fileName": "sample.m4a",
                        "contentType": "audio/mp4",
                        "duration": 2.0
                    }
                })
                .to_string(),
                ..PostFields::default()
            },
        );

        let projection = authority_projection(&event).expect("project audio authority");

        assert_eq!(
            projection.received_data["props"]["file"]["name"],
            "sample.m4a"
        );
        assert!(projection.received_data["props"]["file"]
            .get("fileName")
            .is_none());
        assert_eq!(projection.last_post["props"]["file"]["name"], "sample.m4a");
    }
}

/// 从 createPosts authority 构造逐 target admission 结果。
pub fn batch_result_from_authority(
    req_id: &str,
    body: &Value,
) -> Result<MessageV3Event, crate::ImError> {
    let targets_in = body
        .get("data")
        .and_then(|data| data.get("targets"))
        .and_then(Value::as_array);
    let targets = targets_in
        .into_iter()
        .flatten()
        .filter_map(|target| {
            let channel_id = target.get("channelId")?.as_str()?.trim();
            if channel_id.is_empty() {
                return None;
            }
            let accepted = target.get("status").and_then(Value::as_str) == Some("accepted");
            Some(serde_json::json!({
                "channelId": channel_id,
                "acceptanceStatus": if accepted { "accepted" } else { "failed" },
                "deliveryStatus": if accepted { "pending" } else { "not-applicable" },
                "error": target.get("error").and_then(Value::as_str),
            }))
        })
        .collect::<Vec<_>>();
    let accepted_count = targets
        .iter()
        .filter(|target| target["acceptanceStatus"] == "accepted")
        .count();
    let failed_count = targets.len().saturating_sub(accepted_count);
    let batch_status = match (accepted_count, failed_count) {
        (0, _) => "failed",
        (_, 0) => "success",
        _ => "partial",
    };
    super::encode(
        "im:posts:batch-result",
        serde_json::json!({
            "reqId": req_id,
            "batchStatus": batch_status,
            "acceptedCount": accepted_count,
            "failedCount": failed_count,
            "deliveryAuthority": "ws-post",
            "targets": targets,
        }),
    )
}

/// 构造未获得 target authority 时的批失败事实。
pub fn batch_error(req_id: &str, error: &str) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:posts:batch-result",
        serde_json::json!({
            "reqId": req_id,
            "batchStatus": "failed",
            "acceptedCount": 0,
            "failedCount": 0,
            "deliveryAuthority": "ws-post",
            "targets": [],
            "error": error,
        }),
    )
}

/// 构造 source 本地读取失败时逐 target 的不可投递结果。
pub fn batch_target_error(
    req_id: &str,
    channel_ids: &[String],
    error: &str,
) -> Result<MessageV3Event, crate::ImError> {
    let targets = channel_ids
        .iter()
        .map(|channel_id| {
            serde_json::json!({
                "channelId": channel_id,
                "acceptanceStatus": "failed",
                "deliveryStatus": "not-applicable",
                "error": error,
            })
        })
        .collect::<Vec<_>>();
    super::encode(
        "im:posts:batch-result",
        serde_json::json!({
            "reqId": req_id,
            "batchStatus": "failed",
            "acceptedCount": 0,
            "failedCount": targets.len(),
            "deliveryAuthority": "ws-post",
            "targets": targets,
            "error": error,
        }),
    )
}

/// 构造发送失败事实。
pub fn send_failed(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:send-failed", data)
}

/// 构造按 temporaryId 锚定的本地发送失败事实。
pub fn send_failed_for_identity(
    channel_id: &str,
    temporary_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    send_failed(serde_json::json!({
        "channelId": channel_id,
        "temporaryId": temporary_id,
        "sendStatus": "failed",
    }))
}

/// 构造消息撤回事实。
pub fn revoke(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:revoke", data)
}

/// 从 posts_update authority 构造不含传输元数据的撤回事实。
pub fn revoke_from_authority(
    post: &Value,
    event_seq: u64,
) -> Result<MessageV3Event, crate::ImError> {
    revoke(canonical_revoke_data(post, Some(event_seq))?)
}

/// 从离线 type3 authority 构造同 identity 的最小撤回事实。
pub fn revoke_from_sync_authority(
    event: &crate::sync_session::EventEnvelope,
) -> Result<MessageV3Event, crate::ImError> {
    let id = event
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| crate::ImError::Parse("sync revoke authority missing msgId".to_string()))?;
    revoke(serde_json::json!({
        "id": id,
        "channelId": event.channel_id.as_str(),
        "eventSeq": event.seq.0,
        "revoke": true,
    }))
}

/// 从 update_channel.lastPost authority 构造可持久化的业务 lastPost。
pub fn revoke_last_post_from_authority(post: &Value) -> Result<Value, crate::ImError> {
    canonical_revoke_data(post, None)
}

/// 把 snake/camel 入站别名收敛为单份 MessageV3 撤回字段。
fn canonical_revoke_data(post: &Value, event_seq: Option<u64>) -> Result<Value, crate::ImError> {
    let id = text_alias(post, &["id", "postId", "post_id"])
        .ok_or_else(|| crate::ImError::Parse("revoke authority missing id".to_string()))?;
    let channel_id = text_alias(post, &["channelId", "channel_id"])
        .ok_or_else(|| crate::ImError::Parse("revoke authority missing channelId".to_string()))?;
    let mut data = serde_json::json!({
        "id": id,
        "channelId": channel_id,
        "revoke": true,
    });
    if let Some(event_seq) = event_seq {
        data["eventSeq"] = serde_json::json!(event_seq);
    }
    for (canonical, aliases) in [
        ("temporaryId", &["temporaryId", "temporary_id"][..]),
        ("userId", &["userId", "user_id"][..]),
        ("type", &["type"][..]),
        ("message", &["message"][..]),
        ("createAt", &["createAt", "create_at"][..]),
        ("updateAt", &["updateAt", "update_at"][..]),
    ] {
        if let Some(value) = value_alias(post, aliases) {
            data[canonical] = value.clone();
        }
    }
    Ok(data)
}

/// 按顺序读取字符串别名，拒绝空 identity。
fn text_alias<'a>(value: &'a Value, aliases: &[&str]) -> Option<&'a str> {
    value_alias(value, aliases)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// 按协议优先级读取第一个存在的字段。
fn value_alias<'a>(value: &'a Value, aliases: &[&str]) -> Option<&'a Value> {
    aliases.iter().find_map(|key| value.get(*key))
}

/// 构造消息字段更新事实。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:update", data)
}

/// 从 durable message row 重建 G-06 quickReply 绝对态。
pub fn reaction_update_from_row(
    row: &helix_core::effect::Row,
) -> Result<Option<MessageV3Event>, crate::ImError> {
    reaction::from_row(row)
}

/// 从 durable message row 重建 G-07 expediteMap 绝对态。
pub fn urgent_update_from_row(
    row: &helix_core::effect::Row,
) -> Result<Option<MessageV3Event>, crate::ImError> {
    urgent::from_row(row)
}
