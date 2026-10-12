//! S3 path3 — `post_updates`：新消息触发 channel 写决策（未读 +1 SQL 自增 + lastPost 组）。

use crate::state::ChannelId;

/// path3 决策结果：新消息对 channel 行的更新意图（纯计算，无副作用）。
///
/// 现网 `update_channel_from_post` 两组更新：
/// - **lastPost 组**（`last_root_post_at`/`last_post_at`/`last_post`/`has_schedule_post`）：`visible` 且
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
    /// 本地复核可见性；不可见 post 不应落成 Dialog UI patch。
    pub visible: bool,
    /// 发送者用户 id；userSnapshot.userId 优先，回退 userId。
    pub sender_user_id: String,
    /// 当前 post id，用于 dialog patch / mention_list / urgent_post_list。
    pub post_id: String,
    /// Dialog lastMessage 预览文本。
    pub last_message: String,
    /// 本消息 @ 的 userId 列表。
    pub mentions: Vec<String>,
    /// 本消息加急目标 userId 列表（字段不足时为空）。
    pub urgent_user_ids: Vec<String>,
    /// 当前登录用户是否被 @。
    pub mention_hit: bool,
    /// 当前登录用户是否被加急。
    pub urgent_hit: bool,
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

    // should_increment：sender != auth（自己消息 +0）。无身份 → 计入（不漏未读）。
    let sender = sender_user_id_from_value(data);
    let viewers = string_array(data.get("viewers"));
    let visible = visible_to_user(msg_type, &viewers, auth_user_id)
        || (!auth_user_id.is_empty() && sender == auth_user_id);
    let should_increment = if auth_user_id.is_empty() {
        true
    } else {
        !sender.is_empty() && sender.as_str() != auth_user_id
    };

    let post_id = data
        .get("postId")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let mentions = string_array(data.get("mentions"));
    let (urgent_hit, urgent_user_ids) = urgent_from_value(data, auth_user_id);
    let mention_hit = !auth_user_id.is_empty() && mentions.iter().any(|id| id == auth_user_id);

    let last_post = data.to_string();
    let last_message = data
        .get("simpleMessage")
        .or_else(|| data.get("simple_message"))
        .or_else(|| data.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let has_schedule_post = data
        .get("isSchedule")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let unread_delta = if visible && should_increment { 1 } else { 0 };
    let unread_post_id = if unread_delta > 0 && !post_id.is_empty() {
        Some(post_id.clone())
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
        visible,
        sender_user_id: sender,
        post_id,
        last_message,
        mentions,
        urgent_user_ids,
        mention_hit: visible && mention_hit,
        urgent_hit: visible && urgent_hit,
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
    let visible = visible_to_user(&fields.msg_type, &fields.viewers, auth_user_id)
        || (!auth_user_id.is_empty() && fields.user_id == auth_user_id);

    let should_increment = if auth_user_id.is_empty() {
        true
    } else {
        !fields.user_id.is_empty() && fields.user_id != auth_user_id
    };

    let post_id = fields.id.clone();
    let (urgent_hit, urgent_user_ids) = urgent_from_fields(fields, auth_user_id);
    let mention_hit =
        !auth_user_id.is_empty() && fields.mentions.iter().any(|id| id == auth_user_id);
    let unread_delta = if visible && should_increment { 1 } else { 0 };
    let unread_post_id = if unread_delta > 0 && !post_id.is_empty() {
        Some(post_id.clone())
    } else {
        None
    };

    PostChannelUpdate {
        channel_id,
        unread_delta,
        unread_post_id,
        // sync 路径无整帧 JSON：可见时用消息内容兜底 last_post（守卫命中才落）。
        last_post: if visible {
            sync_last_post_json(fields)
        } else {
            String::new()
        },
        // sync events 不带 isSchedule（排程经独立 schedule action），恒 false。
        has_schedule_post: false,
        msg_create_at: fields.create_at,
        visible,
        sender_user_id: fields.user_id.clone(),
        post_id,
        last_message: if fields.simple_message.is_empty() {
            fields.message.clone()
        } else {
            fields.simple_message.clone()
        },
        mentions: fields.mentions.clone(),
        urgent_user_ids,
        mention_hit: visible && mention_hit,
        urgent_hit: visible && urgent_hit,
    }
}

pub fn message_class(upd: &PostChannelUpdate) -> &'static str {
    match (upd.mention_hit, upd.urgent_hit) {
        (true, true) => "mention_urgent",
        (true, false) => "mention",
        (false, true) => "urgent",
        (false, false) => "normal",
    }
}

/// 标记 type=1 消息事实来自稀疏在线回显或完整同步快照。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PostUpsertSource {
    OnlineSparse,
    SyncSnapshot,
}

/// 将 type=1 权威事实编译为消息与共享频道预览；仅旧在线 post 兼容写 viewer 成员态。
pub(crate) fn message_v3_post_mutation_ops(
    event: &crate::sync_session::EventEnvelope,
    auth_user_id: &str,
    last_post: String,
    source: PostUpsertSource,
) -> Vec<helix_core::effect::StorageOp> {
    use helix_core::effect::{
        BatchUpdateSpec, ScopedGuardedBumpSpec, SqlValue, StorageOp, UpsertSpec,
    };

    let fields = &event.fields;
    let Ok(event_seq) = i64::try_from(event.seq.0) else {
        return Vec::new();
    };
    if auth_user_id.is_empty() {
        return Vec::new();
    }
    let channel_id = event.channel_id.as_str().to_string();
    let viewer_id = auth_user_id.to_string();
    let sender_view = fields.user_id == auth_user_id;
    let should_bump_unread = event
        .unread_bump
        .as_ref()
        .map(|update| update.unread_delta > 0)
        .unwrap_or(!sender_view);
    // 在线 WS post 可能是稀疏 echo；Sync 则持有可覆盖本地行的完整 authority snapshot。
    let message_op = match source {
        PostUpsertSource::OnlineSparse => crate::channel::event_to_online_upsert_op(event),
        PostUpsertSource::SyncSnapshot => crate::channel::event_to_sync_upsert_op(event),
    };
    let mut ops = vec![
        message_op,
        StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.clone())],
            patch: vec![
                ("last_post".to_string(), SqlValue::Text(last_post.clone())),
                (
                    "last_post_at".to_string(),
                    SqlValue::Integer(fields.create_at),
                ),
                (
                    "last_root_post_at".to_string(),
                    SqlValue::Integer(fields.create_at),
                ),
            ],
        }),
    ];
    // Sync 的未读唯一来自同一 authority response 的 memberProjection；消息数量不能反推未读。
    if matches!(source, PostUpsertSource::SyncSnapshot) {
        return ops;
    }
    let member_row = vec![
        ("channel_id".to_string(), SqlValue::Text(channel_id.clone())),
        ("user_id".to_string(), SqlValue::Text(viewer_id.clone())),
        ("unread_count".to_string(), SqlValue::Integer(0)),
        ("last_unread_event_seq".to_string(), SqlValue::Integer(0)),
    ];
    ops.push(StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_member",
        rows: vec![member_row],
        conflict_key: Some("channel_id,user_id"),
        exclude_from_update: vec!["unread_count", "last_unread_event_seq"],
    }));
    ops.push(StorageOp::ScopedGuardedBump(ScopedGuardedBumpSpec {
        table: "channel_member",
        scope_col: "channel_id",
        scope_val: SqlValue::Text(channel_id),
        key_col: "user_id",
        key_val: SqlValue::Text(viewer_id),
        bump_col: "unread_count",
        bump_delta: i64::from(should_bump_unread),
        set_cols: vec![
            (
                "unread_post_id".to_string(),
                SqlValue::Text(if should_bump_unread {
                    fields.id.clone()
                } else {
                    String::new()
                }),
            ),
            ("last_post".to_string(), SqlValue::Text(last_post)),
            (
                "last_post_at".to_string(),
                SqlValue::Integer(fields.create_at),
            ),
            (
                "last_root_post_at".to_string(),
                SqlValue::Integer(fields.create_at),
            ),
            (
                "last_unread_event_seq".to_string(),
                SqlValue::Integer(event_seq),
            ),
        ],
        guard_col: "last_unread_event_seq",
        guard_val: event_seq,
        // 服务端已版本化的成员行只能由 member_projection 推进；POST 不改写同 revision 的摘要/计数。
        additional_guard: Some(("projection_revision", SqlValue::Integer(0))),
    }));
    ops
}

/// 将在线 post authority 与预先投影的 lastPost 编译为含 cursor 的原子写集。
pub fn message_v3_commit_ops(
    event: &crate::sync_session::EventEnvelope,
    auth_user_id: &str,
    last_post: &serde_json::Value,
) -> Vec<helix_core::effect::StorageOp> {
    let mut ops = message_v3_post_mutation_ops(
        event,
        auth_user_id,
        last_post.to_string(),
        PostUpsertSource::OnlineSparse,
    );
    if ops.is_empty() {
        return ops;
    }
    ops.push(crate::acl::to_effect::advance_cursor_op(
        event.channel_id,
        event.seq,
    ));
    ops
}

/// 构造 G-01 当前 viewer 的复合键持久读回。
pub fn message_v3_member_read_op(
    channel_id: ChannelId,
    auth_user_id: &str,
) -> helix_core::effect::StorageOp {
    use helix_core::effect::{ScopedGetSpec, SqlValue, StorageOp};
    StorageOp::ScopedGet(ScopedGetSpec {
        table: "channel_member",
        scope_col: "channel_id",
        scope_val: SqlValue::Text(channel_id.as_str().to_string()),
        key_col: "user_id",
        key_val: SqlValue::Text(auth_user_id.to_string()),
    })
}

mod fields;
use fields::{
    sender_user_id_from_value, string_array, sync_last_post_json, urgent_from_fields,
    urgent_from_value, visible_to_user,
};

#[cfg(test)]
mod tests;
