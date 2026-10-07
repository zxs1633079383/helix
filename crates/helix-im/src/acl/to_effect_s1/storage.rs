//! Channel and member storage effect factories.

use crate::state::ChannelId;
use helix_core::effect::{BatchDeleteSpec, BatchUpdateSpec, Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::Effect;

/// 单列 channel patch（UPDATE channel SET <col>=<val> WHERE id=?）→ `PersistFire`。
///
/// 用于 `channel_close`（delete_at + is_active）/ schedule（has_schedule_post）等定点列写。
/// 本地无此行 → 0 行命中 no-op（对齐现网 update_fields 幂等）。复杂度 O(1)（WHERE PK=id）。
pub fn channel_set_cols(channel_id: ChannelId, cols: Vec<(&'static str, SqlValue)>) -> Effect {
    Effect::PersistFire {
        ops: vec![channel_set_cols_op(channel_id, cols)],
    }
}

/// 返回可放入有回执事务的 channel 定点 patch，供生命周期持久屏障复用。
pub fn channel_set_cols_op(
    channel_id: ChannelId,
    cols: Vec<(&'static str, SqlValue)>,
) -> StorageOp {
    let patch: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "channel",
        key_col: "id",
        key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
        patch,
    })
}

/// 单个成员行（复合 PK channel_id+user_id）。
pub struct MemberRow {
    pub user_id: String,
    pub team_id: String,
    pub role: String,
    pub nick_name: String,
    /// 未提供的字段不参与冲突更新；默认值只用于新行插入。
    pub omitted_fields: Vec<&'static str>,
}

/// `channel_member_update` 的成员行批量 upsert（ON CONFLICT(channel_id,user_id)）→ `PersistFire`。
///
/// 真源：memberChange.join 逐成员入 `channel_member` 表（复合 PK）。`joined_at`/`updated_at`
/// 由 driver ambient 时钟写穿——本工厂只产协议字段列；缺省靠表 DEFAULT。
/// 空成员集 → None（无写意图）。复杂度 O(rows)（逐行单 upsert，每行 O(1)）。
pub fn upsert_channel_members(channel_id: ChannelId, members: Vec<MemberRow>) -> Option<Effect> {
    let ops = upsert_channel_members_ops(channel_id, members);
    (!ops.is_empty()).then_some(Effect::PersistFire { ops })
}

/// 按 presence 分成最多 8 个固定形状批次；同事务写入，复杂度 O(n)。
pub fn upsert_channel_members_ops(
    channel_id: ChannelId,
    members: Vec<MemberRow>,
) -> Vec<StorageOp> {
    let mut groups: std::collections::BTreeMap<u8, Vec<Row>> = std::collections::BTreeMap::new();
    for m in members {
        let mask =
            ["team_id", "role", "nick_name"]
                .iter()
                .enumerate()
                .fold(0u8, |mask, (bit, key)| {
                    mask | if m.omitted_fields.contains(key) {
                        1 << bit
                    } else {
                        0
                    }
                });
        groups.entry(mask).or_default().push(vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.as_str().to_string()),
            ),
            ("user_id".to_string(), SqlValue::Text(m.user_id)),
            ("team_id".to_string(), SqlValue::Text(m.team_id)),
            ("role".to_string(), SqlValue::Text(m.role)),
            ("nick_name".to_string(), SqlValue::Text(m.nick_name)),
        ]);
    }
    groups
        .into_iter()
        .map(|(mask, rows)| {
            StorageOp::BatchUpsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "channel_member",
                rows,
                conflict_key: Some("channel_id,user_id"),
                exclude_from_update: ["team_id", "role", "nick_name"]
                    .into_iter()
                    .enumerate()
                    .filter_map(|(bit, field)| (mask & (1 << bit) != 0).then_some(field))
                    .collect(),
            })
        })
        .collect()
}

/// `memberChange.leave` 的成员离场批量删行（`DELETE FROM channel_member WHERE channel_id=?
/// AND user_id IN(…)`）→ `PersistFire`。
///
/// 真源：`memberChange.leave`（`collect_member_leaves` 解析）→ 从 `channel_member` 表删该 channel
/// 下这些成员。scope=channel_id 作用域约束**绝不跨 channel 误删**（复合 PK 第一段）。
/// 空 leaver 集 → None（无删意图）。复杂度 O(k) 单语句。
pub fn delete_channel_members(channel_id: ChannelId, user_ids: Vec<String>) -> Option<Effect> {
    Some(Effect::PersistFire {
        ops: vec![delete_channel_members_op(channel_id, user_ids)?],
    })
}

/// 返回可并入相关事务的 scoped 成员删除，空离场集保持 no-op。
pub fn delete_channel_members_op(
    channel_id: ChannelId,
    user_ids: Vec<String>,
) -> Option<StorageOp> {
    if user_ids.is_empty() {
        return None;
    }
    Some(StorageOp::BatchDelete(BatchDeleteSpec {
        table: "channel_member",
        scope_col: "channel_id",
        scope_val: SqlValue::Text(channel_id.as_str().to_string()),
        key_col: "user_id",
        key_vals: user_ids.into_iter().map(SqlValue::Text).collect(),
    }))
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
pub fn update_member_nickname_op(
    channel_id: ChannelId,
    user_id: &str,
    nick_name: &str,
) -> StorageOp {
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
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_member",
        rows: vec![row],
        conflict_key: Some("channel_id,user_id"),
        // 冲突时仅改 nick_name：排除其余列（占位插入路径才写它们的 DEFAULT/行内值）。
        exclude_from_update: vec!["team_id", "role"],
    })
}

/// 为无需终态事件的兼容调用方保留 fire-and-forget 昵称写入口。
pub fn update_member_nickname(channel_id: ChannelId, user_id: &str, nick_name: &str) -> Effect {
    Effect::PersistFire {
        ops: vec![update_member_nickname_op(channel_id, user_id, nick_name)],
    }
}
