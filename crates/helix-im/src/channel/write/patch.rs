//! S3 path2 — `collect_present`：`update_channel` 帧 → 仅 `Some` 字段的 PATCH 列集（白名单）。

use super::{ChannelCol, ALLOWED_COLUMNS};
use helix_core::effect::SqlValue;

/// path2 — 从 `update_channel` 帧 data 收 `Some` 字段为 PATCH 列集（真源 `collect_present_fields`）。
///
/// 只收**白名单内**且 WS `Some` 的字段（`None`/未出现 → 跳过 = DB 既有值保留）。
/// 成员 / owner / admin_users / boss 不进此路径（走独立成员表，对齐现网 update_partial filter）。
/// JSON 类（mention_list/urgent_post_list/source/props/picture/target_users）序列化为紧凑字符串。
pub fn collect_present(data: &serde_json::Value) -> Vec<ChannelCol> {
    let mut cols: Vec<ChannelCol> = Vec::new();

    // (DB 列, wire 键)：字符串类。
    const STR_MAP: &[(&str, &str)] = &[
        ("display_name", "displayName"),
        ("type", "type"),
        ("team_id", "teamId"),
        ("create_by", "createBy"),
        ("update_by", "updateBy"),
        ("role", "role"),
        ("orient", "orient"),
        ("purpose", "purpose"),
        ("header", "header"),
        ("root_id", "rootId"),
        ("root_post_id", "rootPostId"),
        ("notify_props", "notifyProps"),
        // Go `update_channel` notify-only frames use the short member key `notify`;
        // normalize it into the viewer-local channel projection column.
        ("notify_props", "notify"),
        ("mention_permission", "mentionPermission"),
        ("notice_permission", "noticePermission"),
        ("top_permission", "topPermission"),
        ("urgent_current_name", "urgentCurrentName"),
        ("picture_type", "pictureType"),
        ("unread_post_id", "unreadPostId"),
    ];
    for (col, key) in STR_MAP {
        if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
            cols.push((col, SqlValue::Text(v.to_string())));
        }
    }

    if let Some(value) = data
        .get("lastPost")
        .or_else(|| data.get("last_post"))
        .filter(|value| !value.is_null())
    {
        let post = crate::message_summary::prepare_post(value);
        let encoded = post
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| post.to_string());
        cols.push(("last_post", SqlValue::Text(encoded)));
    }

    // 整数类（i64 / 计数 / 时间戳）。
    const INT_MAP: &[(&str, &str)] = &[
        ("last_event_seq", "lastEventSeq"),
        ("last_post_at", "lastPostAt"),
        ("delete_at", "deleteAt"),
        ("created_at", "createAt"),
        ("updated_at", "updateAt"),
        ("last_root_post_at", "lastRootPostAt"),
        ("unread_count", "unreadCount"),
        ("mention_count", "mentionCount"),
        ("urgent_count", "urgentCount"),
        ("thread_count", "threadCount"),
        ("top_count", "topCount"),
        ("topic_msg_count", "topicMsgCount"),
        ("admin_max_count", "adminMaxCount"),
        ("mention_count_root", "mentionCountRoot"),
    ];
    for (col, key) in INT_MAP {
        if let Some(v) = data.get(key).and_then(serde_json::Value::as_i64) {
            cols.push((col, SqlValue::Integer(v)));
        }
    }

    // bool 类（落 0/1）。
    const BOOL_MAP: &[(&str, &str)] = &[
        ("is_active", "isActive"),
        ("is_top", "channelIsTop"),
        ("has_more", "hasMore"),
        ("has_urgent_post", "hasUrgentPost"),
        ("has_schedule_post", "hasSchedulePost"),
    ];
    for (col, key) in BOOL_MAP {
        if let Some(v) = data.get(key).and_then(serde_json::Value::as_bool) {
            cols.push((col, SqlValue::Integer(v as i64)));
        }
    }

    // JSON 类（序列化为紧凑字符串；缺省不收）。
    const JSON_MAP: &[(&str, &str)] = &[
        ("mention_list", "mentionList"),
        ("urgent_post_list", "urgentPostList"),
        ("source", "source"),
        ("props", "props"),
        ("picture", "picture"),
        ("target_users", "targetUsers"),
    ];
    for (col, key) in JSON_MAP {
        if let Some(v) = data.get(key).filter(|v| !v.is_null()) {
            // 仅两个历史列表的精确空串规范化，其余 JSON 字段保持原合同。
            let encoded =
                if matches!(*col, "mention_list" | "urgent_post_list") && v.as_str() == Some("") {
                    "[]".into()
                } else {
                    v.to_string()
                };
            cols.push((col, SqlValue::Text(encoded)));
        }
    }

    // 白名单兜底过滤（防越权列；理论上上面映射已全在白名单内，此处双保险）。
    cols.retain(|(col, _)| ALLOWED_COLUMNS.contains(col));
    cols
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(cols: &[ChannelCol], col: &str) -> bool {
        cols.iter().any(|(k, _)| *k == col)
    }
    fn val<'a>(cols: &'a [ChannelCol], col: &str) -> Option<&'a SqlValue> {
        cols.iter().find(|(k, _)| *k == col).map(|(_, v)| v)
    }

    /// 回归锚: S3 path2 —— PATCH 只收 Some 字段，None/缺省跳过（DB 既有值保留）。
    #[test]
    fn collect_present_skips_none_fields_s3() {
        let data = serde_json::json!({
            "displayName": "改名",
            "purpose": "简介",
            "header": "规则",
            "unreadCount": 0,            // Some(0) 显式清零（已读）→ 收
            "channelIsTop": true,
            "mentionList": ["a"],
            // type / lastPost / 其余字段缺省 → 不收
        });
        let cols = collect_present(&data);

        assert!(has(&cols, "display_name"));
        assert!(matches!(val(&cols, "purpose"), Some(SqlValue::Text(s)) if s == "简介"));
        assert!(matches!(val(&cols, "header"), Some(SqlValue::Text(s)) if s == "规则"));
        assert!(
            matches!(val(&cols, "unread_count"), Some(SqlValue::Integer(0))),
            "Some(0) 显式收（清零）"
        );
        assert!(matches!(val(&cols, "is_top"), Some(SqlValue::Integer(1))));
        assert!(matches!(val(&cols, "mention_list"), Some(SqlValue::Text(s)) if s == "[\"a\"]"));
        // None 字段不进 SET
        assert!(!has(&cols, "type"));
        assert!(!has(&cols, "last_post"));
        assert!(!has(&cols, "mention_count"));
    }

    /// 回归锚: S3 path2 —— owner/admin_users/boss/member_count 不进 PATCH（白名单防越权）。
    #[test]
    fn collect_present_excludes_member_and_local_cols_s3() {
        let data = serde_json::json!({
            "owner": { "id": "u1" },
            "adminUsers": [{ "id": "u2" }],
            "boss": [{ "id": "u3" }],
            "memberCount": 9,
            "subtopicsLoadedAt": 100,
            "displayName": "keep",   // 这个该收（对照锚）
        });
        let cols = collect_present(&data);
        assert!(has(&cols, "display_name"));
        for col in [
            "owner",
            "admin_users",
            "boss",
            "member_count",
            "subtopics_loaded_at",
        ] {
            assert!(
                !has(&cols, col),
                "{col} 不进 partial 路径（白名单 / 成员表）"
            );
        }
    }

    /// 回归锚: S3 path2 —— 空 data → 空列集（上层据此 no-op）。
    #[test]
    fn collect_present_empty_s3() {
        assert!(collect_present(&serde_json::json!({})).is_empty());
    }

    /// The notify-only WS spelling must update the same scalar projection as increment sync.
    #[test]
    fn collect_present_accepts_notify_only_wire_key() {
        let cols = collect_present(&serde_json::json!({"notify": "STRONG"}));
        assert!(
            matches!(val(&cols, "notify_props"), Some(SqlValue::Text(value)) if value == "STRONG")
        );
    }
}
