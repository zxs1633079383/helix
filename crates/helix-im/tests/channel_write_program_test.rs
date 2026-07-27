//! 回归锚: S3 path1 —— `channel_write::program_full` 全 54 列 upsert 计算（含 JSON rename + 守卫 omit）。
//!
//! 拆为集成测试（程序逻辑单元）以保 `src/channel_write/program.rs` ≤300 行硬顶（结构规约 §1）。

use helix_core::effect::SqlValue;
use helix_im::channel_write::{program_full, ChannelCol};

fn get<'a>(row: &'a [ChannelCol], col: &str) -> Option<&'a SqlValue> {
    row.iter().find(|(k, _)| *k == col).map(|(_, v)| v)
}

/// 回归锚: S3 path1 —— increment 全量 54 列 + JSON rename + 守卫 omit。
#[test]
fn program_full_emits_54_cols_with_guards_omitted_s3() {
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
        54,
        "path1 = 全 54 列（含 header/member_count 守卫列）"
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

/// 回归锚: S3 path1 —— 非法 id → None（边界零信任）。
#[test]
fn program_full_invalid_id_none_s3() {
    assert!(program_full(&serde_json::json!({ "type": "O" }), "", 0).is_none());
    assert!(program_full(&serde_json::json!({ "id": "short" }), "", 0).is_none());
}
