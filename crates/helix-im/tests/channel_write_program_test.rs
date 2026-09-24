//! 回归锚: S3 path1 —— `channel_write::program_full` 全 55 列 upsert 计算（含 JSON rename + 守卫 omit）。
//!
//! 拆为集成测试（程序逻辑单元）以保 `src/channel_write/program.rs` ≤300 行硬顶（结构规约 §1）。

use helix_core::effect::SqlValue;
use helix_im::channel_write::{program_full, ChannelCol};

fn get<'a>(row: &'a [ChannelCol], col: &str) -> Option<&'a SqlValue> {
    row.iter().find(|(k, _)| *k == col).map(|(_, v)| v)
}

/// Go 的关群快照只写 deleteAt；close=false 不得清掉关闭终态。
#[test]
fn program_full_derives_closed_fact_from_delete_at() {
    for (delete_at, close, expected) in [(123, Some(false), 1), (123, None, 1), (0, Some(false), 0)]
    {
        let mut data =
            serde_json::json!({"id": "00000000000000000000000007", "deleteAt": delete_at});
        if let Some(close) = close {
            data["close"] = serde_json::json!(close);
        }
        let (row, omit) = program_full(&data, "viewer", 1).expect("valid snapshot");
        assert!(
            matches!(get(&row, "is_closed"), Some(SqlValue::Integer(value)) if *value == expected)
        );
        assert_eq!(
            omit.contains(&"is_closed"),
            delete_at == 0 && close.is_none()
        );
    }
}

/// 回归锚: S3 path1 —— increment 全量 55 列 + JSON rename + 守卫 omit。
#[test]
fn program_full_emits_55_cols_with_guards_omitted_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000007",
        "displayName": "群A",
        "type": "O",
        "teamId": "00000000000000000000000001",
        "isActive": true,
        "channelIsTop": true,
        "mentionCount": 3,
        "mentionList": ["x", "y"],
        "lastEventSeq": 42,
        "createAt": 100,
        "updateAt": 200,
        // unread*/purpose/header 缺省 → 应进 omit（回退本地）
    });
    let (row, omit) = program_full(&data, "", 9_999).expect("valid id");

    assert_eq!(
        row.len(),
        55,
        "path1 = 全 55 列（含 header/member_count/is_closed 守卫列）"
    );
    // JSON rename / 透传
    assert!(matches!(get(&row, "display_name"), Some(SqlValue::Text(s)) if s == "群A"));
    assert!(matches!(get(&row, "is_top"), Some(SqlValue::Integer(1))));
    assert!(matches!(
        get(&row, "mention_count"),
        Some(SqlValue::Integer(3))
    ));
    assert!(matches!(get(&row, "mention_list"), Some(SqlValue::Text(s)) if s == "[\"x\",\"y\"]"));
    assert!(matches!(
        get(&row, "last_event_seq"),
        Some(SqlValue::Integer(42))
    ));
    // mention_user 固定空串
    assert!(matches!(get(&row, "mention_user"), Some(SqlValue::Text(s)) if s.is_empty()));
    // 守卫 / 本地 / 占位列进 omit（不覆盖既有）
    for col in [
        "unread_count",
        "unread_post_id",
        "last_post_at",
        "purpose",
        "header",
        "is_remove",
        "draft",
        "members",
        "owner",
        "admin_users",
        "boss",
        "member_count",
        "is_closed",
    ] {
        assert!(omit.contains(&col), "{col} 应进 omit_on_conflict");
    }
    // created_at/updated_at 用 server 值（非 now 回退）
    assert!(matches!(
        get(&row, "created_at"),
        Some(SqlValue::Integer(100))
    ));
    assert!(matches!(
        get(&row, "updated_at"),
        Some(SqlValue::Integer(200))
    ));
}

/// 回归锚: 完整权威帧携带 owner 时必须更新主表 owner，而不是保留冲突行旧值。
#[test]
fn program_full_owner_present_updates_and_is_not_omitted_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000020",
        "owner": {
            "id": "owner-new",
            "teamId": "team-1"
        }
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");

    assert!(matches!(
        get(&row, "owner"),
        Some(SqlValue::Text(value))
            if value == r#"{"id":"owner-new","teamId":"team-1"}"#
    ));
    assert!(
        !omit.contains(&"owner"),
        "权威 owner 存在时必须进入冲突更新集"
    );
}

/// 回归锚: 稀疏帧缺少 owner 时必须保留既有值，不能用占位对象擦除。
#[test]
fn program_full_owner_absent_preserves_existing_on_conflict_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000021",
        "displayName": "稀疏更新"
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");

    assert!(matches!(
        get(&row, "owner"),
        Some(SqlValue::Text(value)) if value == "{}"
    ));
    assert!(omit.contains(&"owner"));
}

/// 回归锚: S3 path1 —— increment 负 urgentCount 不得持久化为负值。
#[test]
fn program_full_clamps_negative_urgent_count_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000014",
        "type": "P",
        "urgentCount": -7,
        "urgentPostList": ["post-14"],
    });
    let (row, _) = program_full(&data, "", 0).expect("valid id");

    assert!(matches!(
        get(&row, "urgent_count"),
        Some(SqlValue::Integer(0))
    ));
    assert!(matches!(
        get(&row, "urgent_post_list"),
        Some(SqlValue::Text(value)) if value == "[\"post-14\"]"
    ));
}

/// 回归锚: 真帧缺 lastPostAt 时以 lastRootPostAt 恢复 lastPost 的目录排序时间。
#[test]
fn program_full_uses_last_root_post_at_for_real_frame_last_post() {
    let data = serde_json::json!({
        "id": "00000000000000000000000012",
        "type": "P",
        "lastPost": "{\"createAt\":1700000000000}",
        "lastRootPostAt": 1700000000000_i64
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");

    assert!(matches!(
        get(&row, "last_post_at"),
        Some(SqlValue::Integer(1700000000000))
    ));
    assert!(
        !omit.contains(&"last_post_at"),
        "lastPost 对应的 lastRootPostAt 必须进入新行和冲突更新集"
    );
}

/// 回归锚: 空 lastPost 不能把缺省 lastRootPostAt 冒充成可见消息排序时间。
#[test]
fn program_full_omits_missing_last_post_at_without_last_post() {
    let data = serde_json::json!({
        "id": "00000000000000000000000013",
        "type": "P",
        "lastRootPostAt": 1700000000000_i64
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");

    assert!(matches!(
        get(&row, "last_post_at"),
        Some(SqlValue::Integer(0))
    ));
    assert!(omit.contains(&"last_post_at"));
}

/// 回归锚: S3 path1 —— created_at/updated_at 缺省回退 now_ms；DM target_users 仅 D 写。
#[test]
fn program_full_now_fallback_and_dm_target_users_s3() {
    let data = serde_json::json!({ "id": "00000000000000000000000008", "type": "O" });
    let (row, _) = program_full(&data, "", 12_345).expect("valid id");
    assert!(matches!(
        get(&row, "created_at"),
        Some(SqlValue::Integer(12345))
    ));
    // 非 DM → target_users 空串
    assert!(matches!(get(&row, "target_users"), Some(SqlValue::Text(s)) if s.is_empty()));

    let dm = serde_json::json!({
        "id": "00000000000000000000000009", "type": "D",
        "targetUsers": ["00000000000000000000000002"]
    });
    let (row, _) = program_full(&dm, "", 0).expect("valid id");
    assert!(
        matches!(get(&row, "target_users"), Some(SqlValue::Text(s)) if s.contains("00000000000000000000000002"))
    );
}

/// 回归锚: S3 path1 —— server 给 memberCount 时落库 + 不 omit（覆盖生效）。
/// 真 Go increment 帧带 memberCount:N（实测 localhost:8065 帧 memberCount 6/9/2/1），
/// 修复前 member_count 整列缺失 → channel.member_count 恒 0（真连 Go 实证 bug）。
#[test]
fn program_full_server_member_count_present_not_omitted_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000010", "type": "P", "memberCount": 9,
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");
    assert!(
        matches!(get(&row, "member_count"), Some(SqlValue::Integer(9))),
        "server Some → 落 member_count=9"
    );
    assert!(
        !omit.contains(&"member_count"),
        "server Some → 不 omit（覆盖生效）"
    );
}

/// 回归锚: S3 path1 —— server 给 unread 时不进 omit（覆盖生效）。
#[test]
fn program_full_server_unread_present_not_omitted_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000010", "type": "O",
        "unreadCount": 5, "unreadPostId": "00000000000000000000000099",
        "purpose": "p", "header": "h"
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid id");
    assert!(matches!(
        get(&row, "unread_count"),
        Some(SqlValue::Integer(5))
    ));
    assert!(
        !omit.contains(&"unread_count"),
        "server Some → 不 omit（覆盖）"
    );
    assert!(!omit.contains(&"unread_post_id"));
    assert!(!omit.contains(&"purpose"));
    assert!(!omit.contains(&"header"));
    assert!(matches!(get(&row, "header"), Some(SqlValue::Text(v)) if v == "h"));
}

/// 回归锚: 稀疏 T 频道快照不得清空 make-topic durable readback 已建立的父频道和根消息关系。
#[test]
fn program_full_topic_omits_empty_root_relation_columns() {
    let data = serde_json::json!({
        "id": "00000000000000000000000015",
        "type": "T",
        "rootId": "",
        "rootPostId": "",
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid topic channel");

    assert!(matches!(get(&row, "root_id"), Some(SqlValue::Text(value)) if value.is_empty()));
    assert!(matches!(get(&row, "root_post_id"), Some(SqlValue::Text(value)) if value.is_empty()));
    assert!(omit.contains(&"root_id"));
    assert!(omit.contains(&"root_post_id"));
}

/// 回归锚: T 频道带权威 root 字段时仍必须更新本地关系，不能被守卫误伤。
#[test]
fn program_full_topic_updates_non_empty_root_relation_columns() {
    let data = serde_json::json!({
        "id": "00000000000000000000000016",
        "type": "T",
        "rootId": "00000000000000000000000017",
        "rootPostId": "00000000000000000000000018",
    });
    let (row, omit) = program_full(&data, "", 0).expect("valid topic channel");

    assert!(
        matches!(get(&row, "root_id"), Some(SqlValue::Text(value)) if value == "00000000000000000000000017")
    );
    assert!(
        matches!(get(&row, "root_post_id"), Some(SqlValue::Text(value)) if value == "00000000000000000000000018")
    );
    assert!(!omit.contains(&"root_id"));
    assert!(!omit.contains(&"root_post_id"));
}

/// 回归锚: S3 path1 —— 非法 id → None（边界零信任）。
#[test]
fn program_full_invalid_id_none_s3() {
    assert!(program_full(&serde_json::json!({ "type": "O" }), "", 0).is_none());
    assert!(program_full(&serde_json::json!({ "id": "short" }), "", 0).is_none());
}
