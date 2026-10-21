//! S3 path1 — `program_full`：`increment_channel` 帧 → 全 55 列 upsert 行（JSON rename + 守卫）。

use super::ChannelCol;
use crate::state::ChannelId;
use helix_core::effect::SqlValue;

/// path1 — 从 `increment_channel` 帧 data 计算全 55 列 upsert 行（含 JSON rename + 守卫覆盖）。
///
/// `auth_user_id`：当前登录用户（写 `user_id` 列；helix-im 无身份 → 传入空串时该列为空，
/// 由 driver/host 注入真值，对齐 emit 路径的 sender 豁免下沉策略）。
/// `now_ms`：`created_at`/`updated_at` 缺省回退（确定性，由 host 经 Clock 注入）。
///
/// **守卫列**（`unread_count`/`unread_post_id`/`purpose`）+ **本地列**（`is_remove`/`draft`）+
/// **占位列**（members/admin_users/boss）始终进 `omit`；owner 仅在 server 缺失时进 `omit`——
/// 明确 owner 会参与 ON CONFLICT 更新，稀疏帧则保留既有值。
/// 返回 `(rows, omit_on_conflict)`。id 非法（缺 / 非 26 字符）→ None（边界零信任）。
pub fn program_full(
    data: &serde_json::Value,
    auth_user_id: &str,
    now_ms: u64,
) -> Option<(Vec<ChannelCol>, Vec<&'static str>)> {
    let id = data
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)?;

    let s =
        |k: &str| -> Option<String> { data.get(k).and_then(|v| v.as_str()).map(str::to_string) };
    let i = |k: &str| -> Option<i64> { data.get(k).and_then(serde_json::Value::as_i64) };
    let b = |k: &str| -> Option<bool> { data.get(k).and_then(serde_json::Value::as_bool) };
    // JSON 子树 → 紧凑字符串（缺省/null 回退默认）。
    let j = |k: &str, dflt: &str| -> String {
        data.get(k)
            .filter(|v| !v.is_null())
            .map(|v| {
                // 旧列表空串不得重新进入持久行；其余字段和非空数据不改写。
                if matches!(k, "mentionList" | "urgentPostList") && v.as_str() == Some("") {
                    "[]".into()
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_else(|| dflt.to_string())
    };

    let ch_type = s("type").unwrap_or_default();
    let mut omit: Vec<&'static str> = Vec::new();
    // 数量更新统一走独立版本门禁；新行保留初值，普通资料覆盖不能改已有数量。
    omit.push("thread_count");
    let mut row: Vec<ChannelCol> = Vec::with_capacity(55);

    row.push(("id", SqlValue::Text(id.as_str().to_string())));
    row.push((
        "display_name",
        SqlValue::Text(s("displayName").unwrap_or_default()),
    ));
    row.push(("type", SqlValue::Text(ch_type.clone())));
    // user_id = 当前登录用户（非 VO 字段）；helix-im 无身份时空串占位（host 注入真值）。
    row.push(("user_id", SqlValue::Text(auth_user_id.to_string())));
    row.push(("team_id", SqlValue::Text(s("teamId").unwrap_or_default())));
    row.push((
        "is_active",
        SqlValue::Integer(b("isActive").unwrap_or(false) as i64),
    ));
    // is_remove 本地列：server 不推 → 不进 ON CONFLICT 更新集（保留本地）；新行取默认 0。
    row.push(("is_remove", SqlValue::Integer(0)));
    omit.push("is_remove");
    row.push((
        "is_top",
        SqlValue::Integer(b("channelIsTop").unwrap_or(false) as i64),
    ));

    // 守卫列：unread_count / unread_post_id（server None → 不进 ON CONFLICT 更新集 = 回退本地）。
    let unread_count = i("unreadCount").or_else(|| i("unread_count"));
    match unread_count {
        Some(v) => row.push(("unread_count", SqlValue::Integer(v))),
        None => {
            row.push(("unread_count", SqlValue::Integer(0)));
            omit.push("unread_count");
        }
    }
    // Go increment canonicalizes to snake_case; keep both historical spellings and explicit
    // empty strings. A zero-count snapshot always clears the anchor, even if an old producer
    // accidentally carries a stale non-empty alias in the same frame.
    let zero_count = unread_count == Some(0);
    match if zero_count {
        Some("")
    } else {
        ["unread_post_id", "unreadPostId", "unReadPostId"]
            .into_iter()
            .find_map(|key| data.get(key).and_then(serde_json::Value::as_str))
    } {
        Some(v) => row.push(("unread_post_id", SqlValue::Text(v.to_owned()))),
        None => {
            row.push(("unread_post_id", SqlValue::Text(String::new())));
            omit.push("unread_post_id");
        }
    }

    row.push((
        "mention_count",
        SqlValue::Integer(i("mentionCount").unwrap_or(0)),
    ));
    row.push(("mention_list", SqlValue::Text(j("mentionList", "[]"))));
    // mention_user 固定空串（真源 channel_state.rs:309）。
    row.push(("mention_user", SqlValue::Text(String::new())));
    row.push((
        "urgent_count",
        SqlValue::Integer(i("urgentCount").unwrap_or(0).max(0)),
    ));
    row.push((
        "urgent_post_list",
        SqlValue::Text(j("urgentPostList", "[]")),
    ));
    let last_post = data
        .get("lastPost")
        .or_else(|| data.get("last_post"))
        .filter(|value| !value.is_null())
        .map(|value| crate::message_summary::prepare_post(value))
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default();
    row.push(("last_post", SqlValue::Text(last_post.clone())));
    // Real increment_channel frames may omit lastPostAt while carrying the same last post boundary as lastRootPostAt.
    let last_post_at = i("lastPostAt").or_else(|| {
        if last_post.is_empty() {
            None
        } else {
            i("lastRootPostAt")
        }
    });
    match last_post_at {
        Some(v) => row.push(("last_post_at", SqlValue::Integer(v))),
        None => {
            // Recovery can persist a newer message before the following
            // increment_channel snapshot arrives. An omitted server timestamp
            // must not erase that durable ordering boundary on conflict.
            row.push(("last_post_at", SqlValue::Integer(0)));
            omit.push("last_post_at");
        }
    }
    let delete_at = i("deleteAt").unwrap_or(0);
    row.push(("delete_at", SqlValue::Integer(delete_at)));
    // Go 关群只写 deleteAt；本地关闭投影不能被快照里的 close=false 清掉。
    match (delete_at > 0, b("close")) {
        (true, _) => row.push(("is_closed", SqlValue::Integer(1))),
        (false, Some(closed)) => row.push(("is_closed", SqlValue::Integer(closed as i64))),
        (false, None) => {
            row.push(("is_closed", SqlValue::Integer(0)));
            omit.push("is_closed");
        }
    }
    row.push(("props", SqlValue::Text(j("props", "{}"))));
    row.push((
        "created_at",
        SqlValue::Integer(i("createAt").unwrap_or(now_ms as i64)),
    ));
    row.push((
        "updated_at",
        SqlValue::Integer(i("updateAt").unwrap_or(now_ms as i64)),
    ));
    // members/admin_users/boss 仍由独立 channel_member 表承载真值，主表只保留兼容占位。
    for (col, dflt) in [("members", ""), ("admin_users", "[]"), ("boss", "[]")] {
        row.push((col, SqlValue::Text(dflt.to_string())));
        omit.push(col);
    }
    // owner 明确到达时必须参与冲突更新；稀疏帧缺失时才保留主表既有值。
    match data.get("owner").filter(|value| !value.is_null()) {
        Some(owner) => row.push(("owner", SqlValue::Text(owner.to_string()))),
        None => {
            row.push(("owner", SqlValue::Text("{}".to_string())));
            omit.push("owner");
        }
    }
    row.push(("source", SqlValue::Text(j("source", ""))));
    // target_users 仅 DM（type=="D"）写否则空串（真源 channel_state.rs:254-262）。
    let target_users = if ch_type == "D" || ch_type.is_empty() {
        j("targetUsers", "")
    } else {
        String::new()
    };
    row.push(("target_users", SqlValue::Text(target_users)));
    row.push(("picture", SqlValue::Text(j("picture", "{}"))));
    row.push((
        "picture_type",
        SqlValue::Text(s("pictureType").unwrap_or_default()),
    ));
    row.push(("orient", SqlValue::Text(s("orient").unwrap_or_default())));
    // purpose 守卫列（server None → 回退本地）。
    match s("purpose") {
        Some(v) => row.push(("purpose", SqlValue::Text(v))),
        None => {
            row.push(("purpose", SqlValue::Text(String::new())));
            omit.push("purpose");
        }
    }
    match s("header") {
        Some(v) => row.push(("header", SqlValue::Text(v))),
        None => {
            row.push(("header", SqlValue::Text(String::new())));
            omit.push("header");
        }
    }
    // draft 本地列：server 不推 → 不进 ON CONFLICT 更新集（保留本地草稿）。
    row.push(("draft", SqlValue::Text(String::new())));
    omit.push("draft");
    row.push(("role", SqlValue::Text(s("role").unwrap_or_default())));
    // make-topic 的 HTTP durable readback 会先建立 T 频道关系；随后到达的稀疏 WS 快照
    // 不得用空 root 字段把该关系抹掉。非 T 频道继续遵循服务端全量覆盖语义。
    for (column, field) in [("root_id", "rootId"), ("root_post_id", "rootPostId")] {
        match s(field).filter(|value| !value.is_empty()) {
            Some(value) => row.push((column, SqlValue::Text(value))),
            None if ch_type == "T" => {
                row.push((column, SqlValue::Text(String::new())));
                omit.push(column);
            }
            None => row.push((column, SqlValue::Text(String::new()))),
        }
    }
    row.push((
        "notify_props",
        SqlValue::Text(s("notifyProps").or_else(|| s("notify")).unwrap_or_default()),
    ));
    row.push((
        "mention_permission",
        SqlValue::Text(s("mentionPermission").unwrap_or_default()),
    ));
    row.push((
        "notice_permission",
        SqlValue::Text(s("noticePermission").unwrap_or_default()),
    ));
    row.push((
        "top_permission",
        SqlValue::Text(s("topPermission").unwrap_or_default()),
    ));
    row.push((
        "last_event_seq",
        SqlValue::Integer(i("lastEventSeq").unwrap_or(0)),
    ));
    row.push((
        "thread_count",
        SqlValue::Integer(i("threadCount").unwrap_or(0)),
    ));
    row.push(("top_count", SqlValue::Integer(i("topCount").unwrap_or(0))));
    let topic_msg_count = i("topicMsgCount")
        .or_else(|| (ch_type == "T").then(|| i("totalMsgCount")).flatten())
        .unwrap_or(0);
    row.push(("topic_msg_count", SqlValue::Integer(topic_msg_count)));
    row.push((
        "admin_max_count",
        SqlValue::Integer(i("adminMaxCount").unwrap_or(0)),
    ));
    row.push((
        "mention_count_root",
        SqlValue::Integer(i("mentionCountRoot").unwrap_or(0)),
    ));
    row.push((
        "has_more",
        SqlValue::Integer(b("hasMore").unwrap_or(false) as i64),
    ));
    row.push((
        "has_urgent_post",
        SqlValue::Integer(b("hasUrgentPost").unwrap_or(false) as i64),
    ));
    row.push((
        "has_schedule_post",
        SqlValue::Integer(b("hasSchedulePost").unwrap_or(false) as i64),
    ));
    row.push((
        "create_by",
        SqlValue::Text(s("createBy").unwrap_or_default()),
    ));
    row.push((
        "update_by",
        SqlValue::Text(s("updateBy").unwrap_or_default()),
    ));
    row.push((
        "last_root_post_at",
        SqlValue::Integer(i("lastRootPostAt").unwrap_or(0)),
    ));
    row.push((
        "urgent_current_name",
        SqlValue::Text(s("urgentCurrentName").unwrap_or_default()),
    ));
    // 守卫列：member_count（server Some 覆盖 / None 回退本地，对齐现网 channel_state.rs:359-361）。
    // 真 Go increment 帧带 memberCount:N → 必须落库；缺 → 不进 ON CONFLICT 更新集（保留本地）。
    match i("memberCount") {
        Some(v) => row.push(("member_count", SqlValue::Integer(v))),
        None => {
            row.push(("member_count", SqlValue::Integer(0)));
            omit.push("member_count");
        }
    }

    // 插入仍使用列默认值；既有行仅覆盖本帧提供且类型有效的字段。
    // 固定 55 列有界比较；派生列沿用上面的专用不变量。
    let provided = super::collect_present(data);
    for &(column, _) in &row {
        if [
            "is_closed",
            "unread_count",
            "unread_post_id",
            "last_post_at",
            "topic_msg_count",
        ]
        .contains(&column)
        {
            continue;
        }
        if super::ALLOWED_COLUMNS.contains(&column)
            && !provided.iter().any(|(key, _)| *key == column)
            && !omit.contains(&column)
        {
            omit.push(column);
        }
    }
    if i("topicMsgCount").is_none() && !(ch_type == "T" && i("totalMsgCount").is_some()) {
        omit.push("topic_msg_count");
    }
    omit.push("mention_user");
    if auth_user_id.is_empty() {
        omit.push("user_id");
    }
    Some((row, omit))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T 频道内部保留 totalMsgCount，dialog renderer 负责禁止把它画成父群话题徽标。
    #[test]
    fn topic_total_message_count_populates_internal_topic_count() {
        let channel_id = crate::state::test_channel_id(81);
        let (row, _) = program_full(
            &serde_json::json!({
                "id": channel_id.as_str(),
                "type": "T",
                "displayName": "话题",
                "totalMsgCount": 11
            }),
            "viewer",
            1,
        )
        .expect("topic channel snapshot");

        let Some((_, SqlValue::Integer(topic_count))) =
            row.iter().find(|(column, _)| *column == "topic_msg_count")
        else {
            panic!("topic_msg_count integer column")
        };
        assert_eq!(*topic_count, 11);
    }
}

/// 未查询话题总数的普通完整投影，不得清掉之前已知的权威数量。
#[cfg(test)]
#[test]
fn absent_topic_count_preserves_existing_authority() {
    for value in [None, Some(0_i64), Some(4_i64)] {
        let mut data = serde_json::json!({"id":"chfixx00000000000000000064", "type":"P"});
        if let Some(count) = value {
            data["threadCount"] = serde_json::json!(count);
        }
        let (_, omit) = program_full(&data, "viewer", 1).expect("valid channel");
        assert!(omit.contains(&"thread_count"));
    }
}
