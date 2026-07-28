//! S3 path1 — `program_full`：`increment_channel` 帧 → 全 54 列 upsert 行（JSON rename + 守卫）。

use super::ChannelCol;
use crate::state::ChannelId;
use helix_core::effect::SqlValue;

/// path1 — 从 `increment_channel` 帧 data 计算全 54 列 upsert 行（含 JSON rename + 守卫覆盖）。
///
/// `auth_user_id`：当前登录用户（写 `user_id` 列；helix-im 无身份 → 传入空串时该列为空，
/// 由 driver/host 注入真值，对齐 emit 路径的 sender 豁免下沉策略）。
/// `now_ms`：`created_at`/`updated_at` 缺省回退（确定性，由 host 经 Clock 注入）。
///
/// **守卫列**（`unread_count`/`unread_post_id`/`purpose`）+ **本地列**（`is_remove`/`draft`）+
/// **占位列**（members/owner/admin_users/boss）：server `None`/本地维护 时进 `omit` 返回——
/// 上层 upsert 的 ON CONFLICT 更新集排除它们（既有值保留 = "回退本地"；新行取行内字面值/默认）。
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
            .map(|v| v.to_string())
            .unwrap_or_else(|| dflt.to_string())
    };

    let ch_type = s("type").unwrap_or_default();
    let mut omit: Vec<&'static str> = Vec::new();
    let mut row: Vec<ChannelCol> = Vec::with_capacity(54);

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
    match i("unreadCount") {
        Some(v) => row.push(("unread_count", SqlValue::Integer(v))),
        None => {
            row.push(("unread_count", SqlValue::Integer(0)));
            omit.push("unread_count");
        }
    }
    match s("unreadPostId") {
        Some(v) => row.push(("unread_post_id", SqlValue::Text(v))),
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
        SqlValue::Integer(i("urgentCount").unwrap_or(0)),
    ));
    row.push((
        "urgent_post_list",
        SqlValue::Text(j("urgentPostList", "[]")),
    ));
    let last_post = s("lastPost").unwrap_or_default();
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
    row.push(("delete_at", SqlValue::Integer(i("deleteAt").unwrap_or(0))));
    row.push(("props", SqlValue::Text(j("props", "{}"))));
    row.push((
        "created_at",
        SqlValue::Integer(i("createAt").unwrap_or(now_ms as i64)),
    ));
    row.push((
        "updated_at",
        SqlValue::Integer(i("updateAt").unwrap_or(now_ms as i64)),
    ));
    // members/owner/admin_users/boss：path1 主表落字面量空值（真源 channel_repo.rs:214-217），
    // 成员真值进独立 channel_member 表。owned 占位 + 不进 ON CONFLICT 更新集（保留既有空值语义）。
    for (col, dflt) in [
        ("members", ""),
        ("owner", "{}"),
        ("admin_users", "[]"),
        ("boss", "[]"),
    ] {
        row.push((col, SqlValue::Text(dflt.to_string())));
        omit.push(col);
    }
    row.push(("source", SqlValue::Text(j("source", ""))));
    // target_users 仅 DM（type=="D"）写否则空串（真源 channel_state.rs:254-262）。
    let target_users = if ch_type == "D" {
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
    row.push(("root_id", SqlValue::Text(s("rootId").unwrap_or_default())));
    row.push((
        "root_post_id",
        SqlValue::Text(s("rootPostId").unwrap_or_default()),
    ));
    row.push((
        "notify_props",
        SqlValue::Text(s("notifyProps").unwrap_or_default()),
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
