use super::*;

fn ch() -> ChannelId {
    crate::state::test_channel_id(7)
}
const AUTH: &str = "00000000000000000000000001";
const OTHER: &str = "00000000000000000000000002";

/// 回归锚: S3 path3 —— 他人可见消息 → unread +1 + 写 unread_post_id。
#[test]
fn post_updates_other_visible_increments_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000099",
        "type": "POST",
        "userId": OTHER,
        "viewers": ["all"],
    });
    let upd = post_updates(ch(), &data, AUTH, 1000);
    assert_eq!(upd.unread_delta, 1, "他人可见消息 +1");
    assert_eq!(
        upd.unread_post_id.as_deref(),
        Some("00000000000000000000000099")
    );
    assert_eq!(upd.msg_create_at, 1000);
    assert!(!upd.last_post.is_empty());
}

/// 回归锚: S3 path3 —— 自己的消息 → +0（sender 豁免），不写 unread_post_id。
#[test]
fn post_updates_self_message_zero_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000099",
        "type": "POST",
        "userId": AUTH,            // sender == auth → 自己
        "viewers": ["all"],
    });
    let upd = post_updates(ch(), &data, AUTH, 1000);
    assert_eq!(upd.unread_delta, 0, "自己的消息 +0");
    assert!(upd.unread_post_id.is_none());
    // 可见 → last_post 仍写（自己发的也要更新会话最后一条）
    assert!(!upd.last_post.is_empty());
}

/// 回归锚: S3 path3 —— 不可见消息（viewers 不含 auth/all）→ +0，last_post 空。
#[test]
fn post_updates_invisible_zero_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000099",
        "type": "POST",
        "userId": OTHER,
        "viewers": ["00000000000000000000000003"],   // 不含 auth / all
    });
    let upd = post_updates(ch(), &data, AUTH, 1000);
    assert_eq!(upd.unread_delta, 0, "不可见 +0");
    assert!(upd.unread_post_id.is_none());
    assert!(upd.last_post.is_empty(), "不可见 → last_post 不写（空）");
}

/// 回归锚: S3 path3 —— userSnapshot.userId 优先于 userId 判 sender。
#[test]
fn post_updates_user_snapshot_priority_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000099",
        "type": "POST",
        "userId": OTHER,                       // 顶层是他人
        "userSnapshot": { "userId": AUTH },    // 快照是自己 → 优先 → 豁免
        "viewers": ["all"],
    });
    let upd = post_updates(ch(), &data, AUTH, 1000);
    assert_eq!(upd.unread_delta, 0, "userSnapshot.userId==auth → 自己 +0");
}

/// 回归锚: S3 path3 —— NOTICE 恒可见；isSchedule → has_schedule_post。
#[test]
fn post_updates_notice_and_schedule_s3() {
    let data = serde_json::json!({
        "id": "00000000000000000000000099",
        "type": "NOTICE",
        "userId": OTHER,
        "isSchedule": true,
        // 无 viewers，但 NOTICE 恒可见
    });
    let upd = post_updates(ch(), &data, AUTH, 1000);
    assert!(!upd.last_post.is_empty(), "NOTICE 恒可见 → last_post 写");
    assert!(upd.has_schedule_post);
    assert_eq!(upd.unread_delta, 1, "NOTICE 他人 → +1");
}
