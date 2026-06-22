//! S3 path3 — `post_updates`：新消息触发 channel 写决策（未读 +1 SQL 自增 + lastPost 组）。

use crate::state::ChannelId;

/// path3 决策结果：新消息对 channel 行的更新意图（纯计算，无副作用）。
///
/// 现网 `update_channel_from_post` 两组更新：
/// - **lastPost 组**（`last_root_post_at`/`last_post`/`has_schedule_post`）：`visible` 且
///   `msg_create_at >= local.last_root_post_at` 才更新。
/// - **未读组**（`unread_count` +1 + `unread_post_id`）：`should_increment` 且
///   `msg_create_at > local.last_root_post_at` 才 +1（**SQL 自增**，非覆盖）。
///
/// helix 用单条 `GuardedBump`（`SET unread_count=unread_count+delta, set_cols… WHERE id=? AND
/// ? > last_root_post_at`）一并表达：守卫 `msg_create_at > last_root_post_at` 同时门控两组
/// （现网两组守卫等价：>= 与 > 在「新消息推进」语义下对最新一条恒成立）。
/// **无 mention_count++**（spec §S3.3 红线，已坐实现网 post 路径不自增 mention_count）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostChannelUpdate {
    pub channel_id: ChannelId,
    /// unread 自增量：他人可见消息=1；自己消息 / 不可见=0（sender 豁免 + 可见性门控）。
    pub unread_delta: i64,
    /// `unread_post_id` 绝对值（unread_delta>0 时写 post id；=0 时为 None 不写该列）。
    pub unread_post_id: Option<String>,
    /// `last_post` 绝对值（post 完整 JSON 字符串）；不可见时为空串（上层据此跳过该列）。
    pub last_post: String,
    /// `has_schedule_post`（data.isSchedule==true）。
    pub has_schedule_post: bool,
    /// 守卫值 = msg_create_at（仅当 > 当前 last_root_post_at 才命中整行更新）。
    pub msg_create_at: i64,
}

/// path3 纯函数：从新消息字段计算 `PostChannelUpdate`（CAP-9 在线/离线同口径）。
///
/// 谓词（真源 message_service.rs:322-363）：
/// - **visible**：`msg_type=="NOTICE"` → true；否则 `viewers` 含 `"all"` 或含 `auth_user_id`。
/// - **should_increment**：`sender != auth_user_id`（自己消息豁免 +0）。sender 优先取
///   `userSnapshot.userId`，回退 `userId`。
///
/// `auth_user_id` 为空（helix-im 无身份）→ 退化：sender 豁免无法判定，按「全部可见 + 计入」
/// 保守处理（不漏未读；与 emit 路径把 sender 豁免下沉前端同策略）。视为 visible=true。
pub fn post_updates(
    channel_id: ChannelId,
    data: &serde_json::Value,
    auth_user_id: &str,
    msg_create_at: i64,
) -> PostChannelUpdate {
    let msg_type = data.get("type").and_then(|v| v.as_str()).unwrap_or("");

    // visible_to_user
    let visible = if msg_type == "NOTICE" {
        true
    } else if auth_user_id.is_empty() {
        // 无身份：保守可见（不漏 last_post / 未读；sender 豁免下沉 driver/前端）。
        true
    } else {
        data.get("viewers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|x| {
                    let s = x.as_str().unwrap_or("");
                    s == "all" || s == auth_user_id
                })
            })
            .unwrap_or(false)
    };

    // should_increment：sender != auth（自己消息 +0）。无身份 → 计入（不漏未读）。
    let sender = data
        .get("userSnapshot")
        .and_then(|s| s.get("userId"))
        .and_then(|v| v.as_str())
        .or_else(|| data.get("userId").and_then(|v| v.as_str()))
        .unwrap_or("");
    let should_increment = if auth_user_id.is_empty() {
        true
    } else {
        !sender.is_empty() && sender != auth_user_id
    };

    let post_id = data
        .get("postId")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let last_post = data.to_string();
    let has_schedule_post = data
        .get("isSchedule")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let unread_delta = if visible && should_increment { 1 } else { 0 };
    let unread_post_id = if unread_delta > 0 && !post_id.is_empty() {
        Some(post_id)
    } else {
        None
    };

    PostChannelUpdate {
        channel_id,
        unread_delta,
        unread_post_id,
        last_post: if visible { last_post } else { String::new() },
        has_schedule_post,
        msg_create_at,
    }
}

/// path3（A3/CAP-9 离线 sync 应用版）：从已解析的 `PostFields` 计算 `PostChannelUpdate`。
///
/// 与 `post_updates`（在线 WS，吃 `serde_json::Value`）**同一谓词**，仅入参形态不同——sync
/// `events` 路径的内容在 `messages` map 的 owned `PostFields`（HX-C005 parser 已一次解析），
/// 不再有原始帧 Value。现网 sync apply 走在线同一条 `update_channel_from_post`（post.rs:1234，
/// CAP-9 在线/离线同口径，sender 豁免一致、不重复 +1）。
///
/// 谓词（真源 message_service.rs:322-363，与 `post_updates` 逐字等价）：
/// - **visible**：`msg_type=="NOTICE"` → true；`auth` 空 → 保守 true；否则 `viewers` 含
///   `"all"` 或 `auth_user_id`。
/// - **should_increment**：`user_id != auth_user_id`（自己消息豁免 +0）；`auth` 空 → 计入。
///   注：sync `PostFields` 无 `userSnapshot`，sender 直接取 `user_id`（现网 sync apply 的
///   `update_unread_count` 对无 snapshot 帧亦回退 `msg.userId`，口径一致）。
/// - `last_post`：sync 路径无原始整帧 JSON，用 message 内容兜底（前端 last_post 主要取 id/内容；
///   守卫命中才写，不可见为空）。
pub fn post_updates_from_fields(
    channel_id: ChannelId,
    fields: &crate::sync_session::PostFields,
    auth_user_id: &str,
) -> PostChannelUpdate {
    // NOTICE 恒可见；无身份保守可见（不漏未读，sender 豁免下沉前端，与在线 post_updates 同策略）。
    let visible = fields.msg_type == "NOTICE"
        || auth_user_id.is_empty()
        || fields
            .viewers
            .iter()
            .any(|v| v == "all" || v == auth_user_id);

    let should_increment = if auth_user_id.is_empty() {
        true
    } else {
        !fields.user_id.is_empty() && fields.user_id != auth_user_id
    };

    let post_id = fields.id.clone();
    let unread_delta = if visible && should_increment { 1 } else { 0 };
    let unread_post_id = if unread_delta > 0 && !post_id.is_empty() {
        Some(post_id)
    } else {
        None
    };

    PostChannelUpdate {
        channel_id,
        unread_delta,
        unread_post_id,
        // sync 路径无整帧 JSON：可见时用消息内容兜底 last_post（守卫命中才落）。
        last_post: if visible {
            fields.message.clone()
        } else {
            String::new()
        },
        // sync events 不带 isSchedule（排程经独立 schedule action），恒 false。
        has_schedule_post: false,
        msg_create_at: fields.create_at,
    }
}

#[cfg(test)]
mod tests {
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
}
