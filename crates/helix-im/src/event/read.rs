//! Read 领域 MessageV3 事件。

use super::MessageV3Event;
use crate::error::ImError;
use serde_json::Value;

/// 构造 viewer 的频道已读绝对态。
pub fn channel(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:read:channel", data)
}

/// 从已持久化的当前成员绝对态构造 G-13 频道已读事件。
pub fn channel_from_member_projection(
    channel_id: &str,
    projection: &Value,
) -> Result<MessageV3Event, crate::ImError> {
    let row = projection.get("dialogPatch").unwrap_or(projection);
    let mut data = serde_json::json!({
        "channelId": channel_id,
        "userId": row.get("userId").cloned().unwrap_or(Value::Null),
        "state": "read",
        "unreadCount": row.get("unreadCount").cloned().unwrap_or(Value::Null),
        "unreadPostId": row.get("unreadPostId").cloned().unwrap_or(Value::Null),
        "mentionCount": row.get("mentionCount").cloned().unwrap_or(Value::Null),
        "mentionCountRoot": row.get("mentionCountRoot").cloned().unwrap_or(Value::Null),
        "mentionList": row.get("mentionList").cloned().unwrap_or(Value::Null),
        "urgentCount": row.get("urgentCount").cloned().unwrap_or(Value::Null),
        "urgentPostList": row.get("urgentPostList").cloned().unwrap_or(Value::Null),
        "lastReadSeq": row.get("lastReadSeq").cloned().unwrap_or(Value::Null),
        "projectionRevision": row.get("projectionRevision").cloned().unwrap_or(Value::Null),
        "lastPost": row.get("lastPost").cloned().unwrap_or(Value::Null),
        "lastPostAt": row.get("lastPostAt").cloned().unwrap_or(Value::Null),
        "lastRootPostAt": row.get("lastRootPostAt").cloned().unwrap_or(Value::Null),
        "msgCount": row.get("msgCount").cloned().unwrap_or(Value::Null),
        "msgCountRoot": row.get("msgCountRoot").cloned().unwrap_or(Value::Null),
        "msgCountPrivate": row.get("msgCountPrivate").cloned().unwrap_or(Value::Null),
    });
    if let Some(count) = row.get("reactionUnreadCount") {
        data["reactionUnreadCount"] = count.clone();
    }
    channel(data)
}

/// 从 dialog 本地查询行恢复当前 viewer 的频道未读绝对态。
pub fn channels_from_storage_rows(
    reply_bytes: &[u8],
    viewer_user_id: &str,
) -> Result<Vec<MessageV3Event>, crate::ImError> {
    let rows = parse_dialog_rows(reply_bytes)?;
    channels_from_dialog_rows(&rows, viewer_user_id)
}

/// 将 dialog storage 回包解析为对象数组，供频道列表与成员回读共享严格边界。
pub(crate) fn parse_dialog_rows(reply_bytes: &[u8]) -> Result<Vec<Value>, crate::ImError> {
    serde_json::from_slice(reply_bytes)
        .map_err(|error| crate::ImError::Parse(format!("dialog read rows: {error}")))
}

/// 从已经验证的 dialog 行生成当前 viewer 的频道未读绝对态事件。
pub(crate) fn channels_from_dialog_rows(
    rows: &[Value],
    viewer_user_id: &str,
) -> Result<Vec<MessageV3Event>, crate::ImError> {
    rows.iter()
        .filter(|row| {
            row.get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
        .map(|row| {
            let channel_id = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let unread_count = integer(&row, "unread_count", "unreadCount");
            let mut data = serde_json::json!({
                "channelId": channel_id,
                "userId": viewer_user_id,
                "state": if unread_count == 0 { "read" } else { "unread" },
                "unreadCount": unread_count,
                "mentionCount": integer(&row, "mention_count", "mentionCount"),
                "urgentCount": integer(&row, "urgent_count", "urgentCount"),
                "lastReadSeq": integer(&row, "last_read_seq", "lastReadSeq"),
                "projectionRevision": integer(&row, "projection_revision", "projectionRevision"),
            });
            if let Some(count) = row
                .get("reaction_unread_count")
                .or_else(|| row.get("reactionUnreadCount"))
            {
                data["reactionUnreadCount"] = count.clone();
            }
            channel(data)
        })
        .collect()
}

/// 成员专属 dialog 字段只覆盖对应投影，不能把成员公司或主键当作频道归属。
const MEMBER_DIALOG_FIELDS: &[(&[&str], &str, &str)] = &[
    (&["last_post", "lastPost"], "last_post", "lastPost"),
    (
        &["last_post_at", "lastPostAt"],
        "last_post_at",
        "lastPostAt",
    ),
    (
        &["last_root_post_at", "lastRootPostAt"],
        "last_root_post_at",
        "lastRootPostAt",
    ),
    (
        &["mention_count", "mentionCount"],
        "mention_count",
        "mentionCount",
    ),
    (
        &["mention_count_root", "mentionCountRoot"],
        "mention_count_root",
        "mentionCountRoot",
    ),
    (
        &["mention_list", "mentionList"],
        "mention_list",
        "mentionList",
    ),
    (
        &["mention_user", "mentionUser"],
        "mention_user",
        "mentionUser",
    ),
    (
        &["urgent_mention_count", "urgent_count", "urgentCount"],
        "urgent_count",
        "urgentCount",
    ),
    (
        &["urgent_post_list", "urgentPostList"],
        "urgent_post_list",
        "urgentPostList",
    ),
    (
        &[
            "urgent_mention_user",
            "urgent_current_name",
            "urgentCurrentName",
        ],
        "urgent_current_name",
        "urgentCurrentName",
    ),
    (&["msg_count", "msgCount"], "msg_count", "msgCount"),
    (
        &["msg_count_root", "msgCountRoot"],
        "msg_count_root",
        "msgCountRoot",
    ),
    (
        &["msg_count_private", "msgCountPrivate"],
        "msg_count_private",
        "msgCountPrivate",
    ),
    (&["effect_id", "effectId"], "effect_id", "effectId"),
];

/// 从成员权威覆盖个人字段；同时替换两种别名，必须在最终整形前调用。
pub(crate) fn overlay_viewer_dialog_projection(
    row: &mut Value,
    member: &Value,
) -> Result<(), ImError> {
    let top = if member
        .get("channel_is_top")
        .or_else(|| member.get("channelIsTop"))
        .is_some()
    {
        Some(dialog_row_bool(member, "channel_is_top", "channelIsTop")?)
    } else {
        None
    };
    let object = row
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("dialog channel row must be object".into()))?;
    for (keys, snake, camel) in MEMBER_DIALOG_FIELDS {
        if let Some(value) = keys.iter().find_map(|key| member.get(*key)) {
            object.insert((*snake).to_string(), value.clone());
            object.insert((*camel).to_string(), value.clone());
        }
    }
    if let Some(role) = member.get("role") {
        object.insert("role".into(), role.clone());
    }
    if let Some(count) = member
        .get("urgent_mention_count")
        .or_else(|| member.get("urgent_count"))
        .or_else(|| member.get("urgentCount"))
        .and_then(Value::as_i64)
    {
        object.insert("has_urgent_post".into(), Value::Bool(count > 0));
        object.insert("hasUrgentPost".into(), Value::Bool(count > 0));
    }
    if let Some(notify) = member
        .get("notify")
        .or_else(|| member.get("notify_props"))
        .or_else(|| member.get("notifyProps"))
    {
        object.insert("notify_props".into(), notify.clone());
        object.insert("notifyProps".into(), notify.clone());
    }
    if let Some(top) = top {
        object.insert("is_top".into(), Value::from(if top { 1 } else { 0 }));
        object.insert("channelIsTop".into(), Value::Bool(top));
    }
    crate::channel_sync::overlay_viewer_unread_projection(row, member)
        .map_err(|()| ImError::Parse("dialog member unread projection invalid".into()))
}

/// 将成员置顶列解析为严格的布尔值，只接受 bool 或 SQLite 0/1。
fn dialog_row_bool(row: &Value, snake: &str, camel: &str) -> Result<bool, ImError> {
    let value = row
        .get(snake)
        .or_else(|| row.get(camel))
        .ok_or_else(|| ImError::Parse("dialog member row missing channel_is_top".into()))?;
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    match value.as_i64() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(ImError::Parse(
            "dialog member channel_is_top must be bool or 0/1".into(),
        )),
    }
}

/// 读取 storage snake_case 或 wire camelCase 的整数字段。
fn integer(row: &Value, snake: &str, camel: &str) -> i64 {
    row.get(snake)
        .or_else(|| row.get(camel))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        })
        .unwrap_or_default()
}

/// 构造单条消息已读事实。
pub fn post(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:read", data)
}

/// 构造消息作者可见的 reader 列表。
pub fn post_readers(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:readers", data)
}

/// 按当前 viewer 与权威 reader 的关系构造 G-14 单视角终态。
pub fn post_for_viewer(
    channel_id: &str,
    post_id: &str,
    author_user_id: &str,
    reader_id: &str,
    snapshot_id: &str,
    member_ids: &[String],
    read_bits: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    post_for_viewer_with_event_seq(
        channel_id,
        post_id,
        author_user_id,
        reader_id,
        snapshot_id,
        member_ids,
        read_bits,
        receipt_revision,
        viewer_user_id,
        None,
    )
}

/// 构造带同事务 type6 序列关联的单视角终态。
///
/// `post_read` 仍是 read_bits 旁路，不凭该字段推进 channel cursor；序列仅用于把
/// legacy authority 与已持久化的 channel_stream_event 对账，并在 render-ready 后保留给客户端。
#[allow(clippy::too_many_arguments)]
pub fn post_for_viewer_with_event_seq(
    channel_id: &str,
    post_id: &str,
    author_user_id: &str,
    reader_id: &str,
    snapshot_id: &str,
    member_ids: &[String],
    read_bits: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
    event_seq: Option<u64>,
) -> Result<MessageV3Event, crate::ImError> {
    let data = serde_json::json!({
        "postId": post_id,
        "channelId": channel_id,
        "authorUserId": author_user_id,
        "readerUserId": reader_id,
        "snapshotId": snapshot_id,
        "memberIds": member_ids,
        "readBits": read_bits,
        "receiptRevision": receipt_revision,
    });
    let mut data = data;
    if let Some(event_seq) = event_seq.filter(|seq| *seq > 0) {
        data["eventSeq"] = serde_json::json!(event_seq);
        data["event_seq"] = serde_json::json!(event_seq);
    }
    if !reader_id.is_empty() && reader_id == viewer_user_id {
        return post(data);
    }
    post_readers(data)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    /// 同一份持久数据同时喂给启动列表与正式同步，隔离查询投影差异，不连接真实数据库。
    fn diagnosis_channel_fixture() -> (Value, Value, crate::query::DialogListScope) {
        let id = crate::state::test_channel_id(1);
        let channel = serde_json::json!({
            "id": id.as_str(), "type": "P", "team_id": "company", "user_id": "viewer",
            "role": "MEMBER", "member_count": 3,
            "members": "", "admin_users": "[]", "boss": "[]", "owner": "",
            "display_name": "group", "purpose": "description", "orient": "orientation",
            "header": "header", "picture": "picture", "picture_type": "CUSTOM",
            "mention_permission": "MANAGER", "notice_permission": "MANAGER", "top_permission": "MANAGER",
            "unread_count": 0, "unread_post_id": "", "mention_count": 0, "mention_list": "[]",
            "urgent_count": 0, "urgent_post_list": "[]", "last_post_at": 10,
            "is_top": 0, "notify_props": "", "updated_at": 20, "last_event_seq": 30,
            "root_id": "root", "root_post_id": "root-post", "topic_msg_count": 4,
            "schedule_id": "schedule", "schedule_status": "pending", "schedule_revision": 6, "has_schedule_post": 1,
            "draft": "{\"text\":\"draft\"}"
        });
        let members = serde_json::json!([
            {"channel_id": id.as_str(), "user_id": "viewer", "team_id": "company", "role": "OWNER",
             "channel_is_top": 1, "notify": "IGNORE", "unread_count": 9, "unread_post_id": "anchor",
             "last_read_seq": 2, "projection_revision": 3, "effect_id": "effect-3",
             "mention_count": 5, "mention_list": "[\"mention\"]", "urgent_mention_count": 3,
             "urgent_post_list": "[\"urgent\"]", "last_post_at": 100},
            {"channel_id": id.as_str(), "user_id": "admin", "team_id": "company", "role": "ADMIN"},
            {"channel_id": id.as_str(), "user_id": "member", "team_id": "company", "role": "MEMBER"}
        ]);
        (
            channel,
            members,
            crate::query::DialogListScope::new("viewer", "company"),
        )
    }

    /// 启动查询在成员组合完成后整形，旧频道角色不得覆盖成员权威。
    #[test]
    fn authority_startup_reads_roster_and_viewer_role_before_shaping() {
        let (channel, members, scope) = diagnosis_channel_fixture();
        let result = crate::channel_sync::project_authority_rows(
            vec![channel],
            members.to_string().as_bytes(),
            &scope,
        )
        .unwrap();
        let row = &result[0];
        assert_eq!(row["adminUsers"].as_array().unwrap().len(), 1);
        assert_eq!(row["role"], "CREATOR");
        assert_eq!(row["canMentionAll"], true);
        assert_eq!(row["canManageMembers"], true);
        assert_eq!(row["unreadCount"], 9);
        assert_eq!(row["mentionCount"], 5);
        assert_eq!(row["urgentCount"], 3);
        assert_eq!(row["lastPostAt"], 100);
        assert_eq!(row["channelIsTop"], true);
        // notify 与角色均属于当前 viewer 成员事实。
        assert_eq!(row["notifyProps"], "IGNORE");
        assert_eq!(row["purpose"], "description");
        assert_eq!(row["orient"], "orientation");
        assert_eq!(row["rootPostId"], "root-post");
        assert_eq!(row["scheduleId"], "schedule");
        assert_eq!(row["scheduleRevision"], 6);
        assert_eq!(row["hasSchedulePost"], true);
        assert_eq!(row["has_schedule_post"], true);
    }

    /// 正式同步必须与启动读取使用同一成员个人投影。
    #[test]
    fn authority_formal_page_overlays_all_viewer_fields() {
        let (channel, members, scope) = diagnosis_channel_fixture();
        let (page, _) = crate::channel_sync::project_page_with_members(
            serde_json::json!([channel]).to_string().as_bytes(),
            members.to_string().as_bytes(),
            &scope,
            0,
        )
        .unwrap();
        let row = &page[0];
        assert!(
            row.get("adminUsers").is_some(),
            "完整成员权威表包含管理员，但 snake_case 占位优先造成 roster 被省略"
        );
        assert_eq!(row["adminUsers"].as_array().unwrap().len(), 1);
        assert_eq!(row["memberCount"], 3);
        assert_eq!(row["role"], "CREATOR");
        assert_eq!(row["canMentionAll"], true);
        assert_eq!(row["unreadCount"], 9);
        // 各入口的个人摘要来自相同成员权威。
        assert_eq!(row["mentionCount"], 5);
        assert_eq!(row["urgentCount"], 3);
        assert_eq!(row["lastPostAt"], 100);
        assert_eq!(row["channelIsTop"], true);
        assert_eq!(row["notifyProps"], "IGNORE");
        assert_eq!(row["purpose"], "description");
        assert_eq!(row["orient"], "orientation");
        assert_eq!(row["rootPostId"], "root-post");
        assert_eq!(row["scheduleId"], "schedule");
    }

    /// 旧 snake_case 容器存在或缺失都不能改变完整成员投影。
    #[test]
    fn authority_roster_does_not_depend_on_stale_alias_presence() {
        let (mut channel, members, scope) = diagnosis_channel_fixture();
        channel.as_object_mut().unwrap().remove("admin_users");
        let (page, _) = crate::channel_sync::project_page_with_members(
            serde_json::json!([channel]).to_string().as_bytes(),
            members.to_string().as_bytes(),
            &scope,
            0,
        )
        .unwrap();
        let row = &page[0];
        assert_eq!(row["adminUsers"].as_array().unwrap().len(), 1);
        assert_eq!(row["memberCount"], 3);
        assert_eq!(row["canMentionAll"], true);
        assert_eq!(row["unreadCount"], 9);
        assert_eq!(row["mentionCount"], 5);
        assert_eq!(row["urgentCount"], 3);
        assert_eq!(row["lastPostAt"], 100);
        assert_eq!(row["channelIsTop"], true);
        assert_eq!(row["notifyProps"], "IGNORE");
    }

    /// 单频道快照共享完整成员投影。
    #[test]
    fn authority_snapshot_shares_complete_projection() {
        let (channel, members, scope) = diagnosis_channel_fixture();
        let result = crate::channel_sync::project_authority_rows(
            vec![channel],
            members.to_string().as_bytes(),
            &scope,
        )
        .unwrap();
        let row = &result[0];
        assert_eq!(row["adminUsers"].as_array().unwrap().len(), 1);
        assert_eq!(row["canMentionAll"], true);
        assert_eq!(row["unreadCount"], 9);
        assert_eq!(row["mentionCount"], 5);
        assert_eq!(row["channelIsTop"], true);
        assert_eq!(row["purpose"], "description");
        assert_eq!(row["scheduleId"], "schedule");
    }

    /// 重启成员绝对态必须携带有效未读锚点与 revision 对应的 read-seq/effect identity。
    #[test]
    fn dialog_merge_uses_member_unread_after_restart() {
        let channel_id = crate::state::test_channel_id(1);
        let channels = vec![serde_json::json!({
            "id": channel_id.as_str(), "team_id": "company", "user_id": "viewer",
            "unread_count": 7
        })];
        let members = serde_json::json!([{
            "channel_id": channel_id.as_str(),
            "user_id": "viewer", "team_id": "company", "role": "MEMBER",
            "channel_is_top": 0,
            "unread_count": 9,
            "unread_post_id": "restart-anchor",
            "last_read_seq": 3,
            "projection_revision": 4,
            "effect_id": "restart-effect-4"
        }]);

        let merged = crate::channel_sync::project_authority_rows(
            channels,
            members.to_string().as_bytes(),
            &crate::query::DialogListScope::new("viewer", "company"),
        )
        .unwrap();

        assert_eq!(merged[0]["unread_count"], 9);
        assert_eq!(merged[0]["projection_revision"], 4);
    }
}
