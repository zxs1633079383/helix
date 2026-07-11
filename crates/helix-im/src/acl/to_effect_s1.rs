//! S1 新增 WS action（编辑 / 已读 / channel 维护 / 成员 / 排程）的 Effect 工厂。
//!
//! 与 `to_effect.rs` 分文件（§1 单文件 ≤300 行硬顶；`to_effect.rs` 现在是 re-export facade）。
//! 职责同 ACL-1：把 IM 概念翻译成 core 通用 Effect/StorageOp——表名 / 列名 / `im:` 前缀作**数据**
//! 经 spec 传给 core，core 不解释（守 HX-C001）。
//!
//! 行为真源：现网 cses-client `handlers/{post,channel,schedule}.rs` + `event_bus.rs:44 emit_im`。

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, DomainEventBytes, Row, SqlValue, StorageOp, UpsertSpec,
};
use helix_core::Effect;

/// 把 `{event, data}` JSON 包成 Emit Effect（信封铁律：与既有 emit_* 同形态）。
///
/// `data` 由调用方按现网 emit payload 形态拼好（如 `{channelId, deleteAt}`）。静态 shape，
/// 序列化失败=编程错（`expect`，非边界输入；边界输入零信任已在 handler 解析阶段完成）。
fn emit(event: &'static str, data: serde_json::Value) -> Effect {
    use serde_json::json;
    let payload = json!({ "event": event, "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_s1: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

// ─── channel 维护事件（created / closed / member-updated / memberNickname）──────

/// `channel_created` → `im:channel:created`（真源 channel.rs:569）。
///
/// 透传帧 `data` 原始字节（含 owner/members/picture 等，前端直接消费）；非法 JSON 降级 null。
pub fn emit_channel_created(channel_id: ChannelId, raw: &[u8]) -> Effect {
    let channel: serde_json::Value = serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    let display_name = channel
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(channel_id.as_str());
    let group = match channel.get("type").and_then(serde_json::Value::as_str) {
        Some("T") => "topic",
        Some("D") => "direct",
        _ => "group",
    };
    let member_count = channel
        .get("memberCount")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    emit(
        "im:channel:created",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "channel_id": channel_id.as_str(),
            "displayName": display_name,
            "group": group,
            "memberCount": member_count,
            "props": channel,
            "channel": channel,
        }),
    )
}

/// `channel_close` → `im:channel:closed`（真源 channel.rs:593 `{channelId, deleteAt}`）。
pub fn emit_channel_closed(channel_id: ChannelId, delete_at: i64) -> Effect {
    emit(
        "im:channel:closed",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "closed": true,
            "deleteAt": delete_at,
        }),
    )
}

/// 自己退群成功的 render-ready 终态；与 channel closed 严格区分。
pub fn emit_channel_left(channel_id: ChannelId) -> Effect {
    emit(
        "im:channel:left",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "left": true,
        }),
    )
}

/// `channel_member_update` → `im:channel:member-updated`（真源 channel.rs:626）。
///
/// 透传帧 `data`（含 memberChange）原始字节；非法 JSON 降级 null。
pub fn emit_channel_member_updated(channel_id: ChannelId, raw: &[u8]) -> Effect {
    let channel: serde_json::Value = serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    emit(
        "im:channel:member-updated",
        serde_json::json!({ "channel_id": channel_id.as_str(), "channel": channel }),
    )
}

/// `update_channel_member_nickName` → `im:channel:memberNickname`（真源 channel.rs:757）。
pub fn emit_member_nickname(channel_id: ChannelId, user_id: &str, nick_name: &str) -> Effect {
    emit(
        "im:channel:memberNickname",
        serde_json::json!({
            "channelId": channel_id.as_str(),
            "userId": user_id,
            "nickName": nick_name,
        }),
    )
}

// ─── 排程事件（schedule created / canceled）────────────────────────────────────

/// `post_schedule_created` → `im:channel:schedule-created`（真源 schedule.rs:25）。
pub fn emit_schedule_created(channel_id: ChannelId) -> Effect {
    emit(
        "im:channel:schedule-created",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "hasSchedulePost": true,
        }),
    )
}

/// `post_schedule_canceled` → `im:channel:schedule-canceled`（真源 schedule.rs:49）。
pub fn emit_schedule_canceled(channel_id: ChannelId) -> Effect {
    emit(
        "im:channel:schedule-canceled",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "hasSchedulePost": false,
        }),
    )
}

// ─── StorageOp 工厂（channel 列 patch / channel_member 行写）────────────────────

/// 单列 channel patch（UPDATE channel SET <col>=<val> WHERE id=?）→ `PersistFire`。
///
/// 用于 `channel_close`（delete_at + is_active）/ schedule（has_schedule_post）等定点列写。
/// 本地无此行 → 0 行命中 no-op（对齐现网 update_fields 幂等）。复杂度 O(1)（WHERE PK=id）。
pub fn channel_set_cols(channel_id: ChannelId, cols: Vec<(&'static str, SqlValue)>) -> Effect {
    let patch: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Effect::PersistFire {
        ops: vec![StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
            patch,
        })],
    }
}

/// 单个成员行（复合 PK channel_id+user_id）。
pub struct MemberRow {
    pub user_id: String,
    pub team_id: String,
    pub role: String,
    pub nick_name: String,
}

/// `channel_member_update` 的成员行批量 upsert（ON CONFLICT(channel_id,user_id)）→ `PersistFire`。
///
/// 真源：memberChange.join 逐成员入 `channel_member` 表（复合 PK）。`joined_at`/`updated_at`
/// 由 driver ambient 时钟写穿——本工厂只产协议字段列；缺省靠表 DEFAULT。
/// 空成员集 → None（无写意图）。复杂度 O(rows)（逐行单 upsert，每行 O(1)）。
pub fn upsert_channel_members(channel_id: ChannelId, members: Vec<MemberRow>) -> Option<Effect> {
    if members.is_empty() {
        return None;
    }
    let cid = channel_id.as_str().to_string();
    let rows: Vec<Row> = members
        .into_iter()
        .map(|m| {
            vec![
                ("channel_id".to_string(), SqlValue::Text(cid.clone())),
                ("user_id".to_string(), SqlValue::Text(m.user_id)),
                ("team_id".to_string(), SqlValue::Text(m.team_id)),
                ("role".to_string(), SqlValue::Text(m.role)),
                ("nick_name".to_string(), SqlValue::Text(m.nick_name)),
            ]
        })
        .collect();
    Some(Effect::PersistFire {
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "channel_member",
            rows,
            conflict_key: Some("channel_id,user_id"),
            exclude_from_update: Vec::new(),
        })],
    })
}

/// `memberChange.leave` 的成员离场批量删行（`DELETE FROM channel_member WHERE channel_id=?
/// AND user_id IN(…)`）→ `PersistFire`。
///
/// 真源：`memberChange.leave`（`collect_member_leaves` 解析）→ 从 `channel_member` 表删该 channel
/// 下这些成员。scope=channel_id 作用域约束**绝不跨 channel 误删**（复合 PK 第一段）。
/// 空 leaver 集 → None（无删意图）。复杂度 O(k) 单语句。
pub fn delete_channel_members(channel_id: ChannelId, user_ids: Vec<String>) -> Option<Effect> {
    if user_ids.is_empty() {
        return None;
    }
    Some(Effect::PersistFire {
        ops: vec![StorageOp::BatchDelete(BatchDeleteSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(channel_id.as_str().to_string()),
            key_col: "user_id",
            key_vals: user_ids.into_iter().map(SqlValue::Text).collect(),
        })],
    })
}

/// `update_channel_member_nickName` 定点改昵称（写 `channel_member` 表，复合 PK，spec §S1）。
///
/// 现网走 `channel_member_repo::update_nickname`（UPDATE … WHERE channel_id=? AND user_id=?，
/// 缺行 affected==0 跳过）。helix-core 的 `BatchUpdate` 仅 `WHERE key_col IN (…)`（单列），
/// **无法**表达复合 `WHERE channel_id=? AND user_id=?`——若用 `key_col="channel_id"` 会误改该
/// channel **全部**成员的 nick_name（漏 user_id 约束，不可接受）。
///
/// 故用复合 PK upsert（`ON CONFLICT(channel_id,user_id) DO UPDATE SET nick_name=excluded`，
/// `exclude_from_update` 排除 role/team_id → 冲突时**仅**改 nick_name，既有 role/team 保留）。
/// 与现网唯一差异：本地缺该成员行时 helix **插一条占位**（PK + nick_name，role 取表 DEFAULT
/// 'MEMBER'），现网则跳过——下次 path1 全量 memberChange 覆盖补齐其余列，权威收敛一致
/// （对齐「成员关系以 increment 全量为准」）。复杂度 O(1)（单行 upsert WHERE 复合 PK）。
pub fn update_member_nickname(channel_id: ChannelId, user_id: &str, nick_name: &str) -> Effect {
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(channel_id.as_str().to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        (
            "nick_name".to_string(),
            SqlValue::Text(nick_name.to_string()),
        ),
    ];
    Effect::PersistFire {
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "channel_member",
            rows: vec![row],
            conflict_key: Some("channel_id,user_id"),
            // 冲突时仅改 nick_name：排除其余列（占位插入路径才写它们的 DEFAULT/行内值）。
            exclude_from_update: vec!["team_id", "role"],
        })],
    }
}
