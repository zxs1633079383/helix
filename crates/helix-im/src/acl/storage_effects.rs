//! Storage Effect factories.

use crate::state::{ChannelId, Seq, TemporaryId};
use helix_core::effect::{MonotonicUpsertSpec, Row, StorageOp, UpsertSpec};
use helix_core::{Correlation, Effect};

/// Canonical, bounded recovery evidence for exactly one remote event. `event_hash` is calculated
/// by the parser over its normalized fields; raw WS/PG JSON is deliberately not persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryLedgerEntry {
    pub tenant_id: String,
    pub channel_id: String,
    pub event_seq: u64,
    pub event_kind: String,
    pub message_id: Option<String>,
    pub event_hash: String,
    pub coverage_id: String,
    pub applied_at_ms: i64,
}

/// One committed contiguous range. The caller owns canonical fact hashing and correlation;
/// this module only converts it to one O(1) keyed upsert operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCoverage {
    pub coverage_id: String,
    pub tenant_id: String,
    pub channel_id: String,
    pub from_seq: u64,
    pub to_seq: u64,
    pub event_count: u64,
    pub facts_hash: String,
    pub correlation_id: String,
    pub committed_at_ms: i64,
}

pub fn recovery_ledger_op(entry: RecoveryLedgerEntry) -> StorageOp {
    use helix_core::effect::SqlValue;
    StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_event_ledger",
        rows: vec![vec![
            ("tenant_id".to_string(), SqlValue::Text(entry.tenant_id)),
            ("channel_id".to_string(), SqlValue::Text(entry.channel_id)),
            ("event_seq".to_string(), SqlValue::Integer(entry.event_seq as i64)),
            ("event_kind".to_string(), SqlValue::Text(entry.event_kind)),
            ("message_id".to_string(), entry.message_id.map_or(SqlValue::Null, SqlValue::Text)),
            ("event_hash".to_string(), SqlValue::Text(entry.event_hash)),
            ("coverage_id".to_string(), SqlValue::Text(entry.coverage_id)),
            ("applied_at_ms".to_string(), SqlValue::Integer(entry.applied_at_ms)),
        ]],
        conflict_key: Some("tenant_id,channel_id,event_seq"),
        exclude_from_update: Vec::new(),
    })
}

pub fn recovery_coverage_op(coverage: RecoveryCoverage) -> StorageOp {
    use helix_core::effect::SqlValue;
    StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_sync_coverage",
        rows: vec![vec![
            ("coverage_id".to_string(), SqlValue::Text(coverage.coverage_id)),
            ("tenant_id".to_string(), SqlValue::Text(coverage.tenant_id)),
            ("channel_id".to_string(), SqlValue::Text(coverage.channel_id)),
            ("from_seq".to_string(), SqlValue::Integer(coverage.from_seq as i64)),
            ("to_seq".to_string(), SqlValue::Integer(coverage.to_seq as i64)),
            ("event_count".to_string(), SqlValue::Integer(coverage.event_count as i64)),
            ("facts_hash".to_string(), SqlValue::Text(coverage.facts_hash)),
            ("correlation_id".to_string(), SqlValue::Text(coverage.correlation_id)),
            ("commit_state".to_string(), SqlValue::Text("committed".to_string())),
            ("committed_at_ms".to_string(), SqlValue::Integer(coverage.committed_at_ms)),
        ]],
        conflict_key: Some("coverage_id"),
        exclude_from_update: Vec::new(),
    })
}

/// 乐观落库：INSERT INTO messages ON CONFLICT(temporary_id) DO UPDATE
///
/// `conflict_key = "temporary_id"` 是 IM 业务键名，由此函数提供给 core，
/// core 只把它作为参数值传递，不理解其含义。
pub fn upsert_message(_temporary_id: &TemporaryId, row: Row, corr: Correlation) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "message",
            rows: vec![row],
            conflict_key: Some("temporary_id"),
            exclude_from_update: Vec::new(),
        })],
    }
}

/// S3 path1：channel 全量 upsert（INSERT … ON CONFLICT(id) DO UPDATE，52 列）。
pub fn upsert_channel_full(
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
    exclude_from_update: Vec<&'static str>,
) -> Effect {
    let row: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Effect::PersistFire {
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "channel",
            rows: vec![row],
            conflict_key: Some("id"),
            exclude_from_update,
        })],
    }
}

/// S3 path2/3：channel 部分更新（UPDATE channel SET <cols> WHERE id=?）。
pub fn update_channel_partial(
    channel_id: ChannelId,
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
) -> Option<Effect> {
    if cols.is_empty() {
        return None;
    }
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let patch: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Some(Effect::PersistFire {
        ops: vec![StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
            patch,
        })],
    })
}

/// `update_channel` 后端 per-member 绝对态补丁 → `channel_member` 复合 PK upsert。
///
/// 这是「当前登录用户在某 channel 的 dialog badge 真值」，不是全群 `channel` 公共字段。
/// Go 后端会按可见性/mention/urgent 计算 member 维度计数后定向广播 `update_channel`，
/// Helix 只覆盖帧中出现的字段，不把 member 计数写回 channel 行。
pub fn upsert_channel_member_dialog(row: Row) -> Option<Effect> {
    if row.is_empty() {
        return None;
    }
    Some(Effect::PersistFire {
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "channel_member",
            rows: vec![row],
            conflict_key: Some("channel_id,user_id"),
            exclude_from_update: Vec::new(),
        })],
    })
}

/// S3 path3：新消息触发 channel 写——未读 +1 SQL 自增 + lastPost 组。
pub fn bump_channel_unread(upd: &crate::channel_write::PostChannelUpdate) -> Effect {
    Effect::PersistFire {
        ops: vec![bump_channel_unread_op(upd)],
    }
}

/// `bump_channel_unread` 的 StorageOp 内核，供 sync 应用路径并入同一 Persist 批。
pub fn bump_channel_unread_op(upd: &crate::channel_write::PostChannelUpdate) -> StorageOp {
    use helix_core::effect::{GuardedBumpSpec, SqlValue};
    let mut set_cols: Row = Vec::with_capacity(3);
    if let Some(ref pid) = upd.unread_post_id {
        set_cols.push(("unread_post_id".to_string(), SqlValue::Text(pid.clone())));
    }
    if !upd.last_post.is_empty() {
        set_cols.push((
            "last_post".to_string(),
            SqlValue::Text(upd.last_post.clone()),
        ));
    }
    if upd.has_schedule_post {
        set_cols.push(("has_schedule_post".to_string(), SqlValue::Integer(1)));
    }
    if upd.mention_hit && !upd.post_id.is_empty() {
        set_cols.push((
            "mention_list".to_string(),
            SqlValue::Text(serde_json::json!([upd.post_id]).to_string()),
        ));
        set_cols.push((
            "mention_user".to_string(),
            SqlValue::Text(serde_json::json!(upd.mentions).to_string()),
        ));
    }
    if upd.urgent_hit && !upd.post_id.is_empty() {
        set_cols.push((
            "urgent_post_list".to_string(),
            SqlValue::Text(serde_json::json!([upd.post_id]).to_string()),
        ));
        set_cols.push(("has_urgent_post".to_string(), SqlValue::Integer(1)));
    }
    set_cols.push((
        "last_root_post_at".to_string(),
        SqlValue::Integer(upd.msg_create_at),
    ));
    let mut extra_bumps = Vec::new();
    if upd.mention_hit {
        extra_bumps.push(("mention_count", 1));
    }
    if upd.urgent_hit {
        extra_bumps.push(("urgent_count", 1));
    }
    StorageOp::GuardedBump(GuardedBumpSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text(upd.channel_id.as_str().to_string()),
        bump_col: "unread_count",
        bump_delta: upd.unread_delta,
        extra_bumps,
        set_cols,
        guard_col: "last_root_post_at",
        guard_val: upd.msg_create_at,
    })
}

/// 读取写后的 channel 投影行。通常跟在 `GuardedBump` 之后放入同一 `Persist{corr}` 批次，
/// 由 driver 顺序执行并把最后一个 `Get` 的 row 回给 Helix 组装累计 DialogPatch。
pub fn get_channel_row_op(channel_id: ChannelId) -> StorageOp {
    use helix_core::effect::{GetSpec, SqlValue};
    StorageOp::Get(GetSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text(channel_id.as_str().to_string()),
    })
}

/// advance_cursor：推进 per-channel 同步 cursor（fire-and-forget，写成功后调用）。
pub fn advance_cursor(channel_id: ChannelId, target_seq: Seq) -> Effect {
    Effect::PersistFire {
        ops: vec![advance_cursor_op(channel_id, target_seq)],
    }
}

/// `advance_cursor` 的 StorageOp 内核，供 sync/increment 与业务写并入同一相关事务。
pub fn advance_cursor_op(channel_id: ChannelId, target_seq: Seq) -> StorageOp {
    StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
        table: "channel_event_cursor",
        key_col: "channel_id",
        value_col: "last_event_seq",
        touch_col: Some("updated_at"),
        scope_key: channel_id.as_str().to_string(),
        value: target_seq.0 as i64,
    })
}

/// type7 `closed` terminal 的本地 tombstone + cursor 逻辑投影。
///
/// 两个水位共存于 `channel_event_cursor` 同一行：`last_event_seq` 是通常 cursor，
/// `terminal_event_seq` 是只表示 closed 的 marker。调用方必须用 `Effect::PersistAtomic`
/// 兑现本 op；普通 `advance_cursor_op` 不触碰 terminal 列，因此无法把 closed marker 清掉。
///
/// 不创建或伪造 Go 的 `channel_event` 表，也不落 `payload` 的任意文本。严格 wire 已证明唯一
/// 合法状态是 `closed`，所以 marker 值本身就是完整的本地投影。
pub fn terminal_tombstone_and_cursor_op(channel_id: ChannelId, target_seq: Seq) -> StorageOp {
    use helix_core::effect::SqlValue;
    StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_event_cursor",
        rows: vec![vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.as_str().to_string()),
            ),
            (
                "last_event_seq".to_string(),
                SqlValue::Integer(target_seq.0 as i64),
            ),
            (
                "terminal_event_seq".to_string(),
                SqlValue::Integer(target_seq.0 as i64),
            ),
        ]],
        conflict_key: Some("channel_id"),
        exclude_from_update: Vec::new(),
    })
}

/// too_long 覆盖式重拉：重置 channel 的 Dialog 派生字段，但保留本地 message 历史。
pub fn reset_channel_dialog_op(channel_id: ChannelId) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "channel",
        key_col: "id",
        key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
        patch: vec![
            ("last_post".to_string(), SqlValue::Text(String::new())),
            ("unread_post_id".to_string(), SqlValue::Text(String::new())),
            ("unread_count".to_string(), SqlValue::Integer(0)),
            ("mention_count".to_string(), SqlValue::Integer(0)),
            ("mention_list".to_string(), SqlValue::Text(String::new())),
            ("mention_user".to_string(), SqlValue::Text(String::new())),
            ("urgent_count".to_string(), SqlValue::Integer(0)),
            (
                "urgent_post_list".to_string(),
                SqlValue::Text(String::new()),
            ),
            ("has_urgent_post".to_string(), SqlValue::Integer(0)),
            ("has_more".to_string(), SqlValue::Integer(1)),
        ],
    })
}
