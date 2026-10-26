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

/// `channel_event_ledger` predates a separate actor column.  Its tenant key is
/// therefore an opaque, length-prefixed tenant+actor scope rather than a plain
/// company id.  This stays collision-free without a destructive primary-key
/// migration and aligns with the native store path's same two-part boundary.
pub fn recovery_tenant_actor_scope(tenant_id: &str, actor_id: &str) -> Option<String> {
    if tenant_id.is_empty() || actor_id.is_empty() {
        return None;
    }
    Some(format!("{}:{}:{}", tenant_id.len(), tenant_id, actor_id))
}

/// Persist the recovery ledger under its declared sparse identity.
pub fn recovery_ledger_op(entry: RecoveryLedgerEntry) -> StorageOp {
    use helix_core::effect::SqlValue;
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_event_ledger",
        rows: vec![vec![
            ("tenant_id".to_string(), SqlValue::Text(entry.tenant_id)),
            ("channel_id".to_string(), SqlValue::Text(entry.channel_id)),
            (
                "event_seq".to_string(),
                SqlValue::Integer(entry.event_seq as i64),
            ),
            ("event_kind".to_string(), SqlValue::Text(entry.event_kind)),
            (
                "message_id".to_string(),
                entry.message_id.map_or(SqlValue::Null, SqlValue::Text),
            ),
            ("event_hash".to_string(), SqlValue::Text(entry.event_hash)),
            ("coverage_id".to_string(), SqlValue::Text(entry.coverage_id)),
            (
                "applied_at_ms".to_string(),
                SqlValue::Integer(entry.applied_at_ms),
            ),
        ]],
        conflict_key: Some("tenant_id,channel_id,event_seq"),
        exclude_from_update: Vec::new(),
    })
}

/// Persist channel recovery coverage without changing legacy update behavior.
pub fn recovery_coverage_op(coverage: RecoveryCoverage) -> StorageOp {
    use helix_core::effect::SqlValue;
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_sync_coverage",
        rows: vec![vec![
            (
                "coverage_id".to_string(),
                SqlValue::Text(coverage.coverage_id),
            ),
            ("tenant_id".to_string(), SqlValue::Text(coverage.tenant_id)),
            (
                "channel_id".to_string(),
                SqlValue::Text(coverage.channel_id),
            ),
            (
                "from_seq".to_string(),
                SqlValue::Integer(coverage.from_seq as i64),
            ),
            (
                "to_seq".to_string(),
                SqlValue::Integer(coverage.to_seq as i64),
            ),
            (
                "event_count".to_string(),
                SqlValue::Integer(coverage.event_count as i64),
            ),
            (
                "facts_hash".to_string(),
                SqlValue::Text(coverage.facts_hash),
            ),
            (
                "correlation_id".to_string(),
                SqlValue::Text(coverage.correlation_id),
            ),
            (
                "commit_state".to_string(),
                SqlValue::Text("committed".to_string()),
            ),
            (
                "committed_at_ms".to_string(),
                SqlValue::Integer(coverage.committed_at_ms),
            ),
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
            version_column: None,
            update_guard: None,
            table: "message",
            rows: vec![row],
            conflict_key: Some("temporary_id"),
            exclude_from_update: Vec::new(),
        })],
    }
}

/// 完整/稀疏快照共享同一个持久版本守卫，其他投影保留各自写入合同。
pub fn upsert_channel_full(
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
    exclude: Vec<&'static str>,
) -> Effect {
    Effect::PersistFire {
        ops: upsert_channel_full_ops(cols, exclude),
    }
}

/// 创建只在类型明确时发生；共享列冲突更新交给独立原子版本比较。
pub fn upsert_channel_full_ops(
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
    mut exclude: Vec<&'static str>,
) -> Vec<StorageOp> {
    use super::channel_snapshot::{is_shared, revision, shared_patch};
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    // INSERT与条件UPDATE需要各自owned字段；只复制共享子集，数量不随成员数增长。
    let shared_keys: Vec<_> = cols
        .iter()
        .filter(|(k, _)| is_shared(k) && !exclude.contains(k))
        .map(|(k, _)| *k)
        .collect();
    let mut row: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let Some(id) = row.iter().find(|(k, _)| k == "id").map(|(_, v)| v.clone()) else {
        return vec![];
    };
    let version = revision(&row);
    let typed = !exclude.contains(&"type")
        && row
            .iter()
            .any(|(k, v)| k == "type" && matches!(v, SqlValue::Text(t) if !t.is_empty()));
    let shared: Row = row
        .iter()
        .filter(|(k, _)| shared_keys.contains(&k.as_str()))
        .cloned()
        .collect();
    let mut ops = Vec::with_capacity(2);
    if typed {
        // 原始INSERT列仍供scope追踪使用；冲突时共享列全部排除，随后由版本门禁写入。
        row.retain(|(k, _)| k != "snapshot_updated_at");
        exclude.extend(shared_keys);
        ops.push(StorageOp::BatchUpsert(UpsertSpec {
            table: "channel",
            rows: vec![row],
            conflict_key: Some("id"),
            exclude_from_update: exclude,
            update_guard: None,
            version_column: None,
        }));
    } else {
        let patch: Row = row
            .into_iter()
            .filter(|(k, _)| {
                k != "id"
                    && k != "snapshot_updated_at"
                    && !is_shared(k)
                    && !exclude.contains(&k.as_str())
            })
            .collect();
        // 空稀疏批次仍保留提交屏障，供increment分页推进与失败回调使用。
        ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![id.clone()], // hot-path-audit: ignore — 固定频道主键跨两个owned操作复用，不复制消息载荷。
            patch,
        }));
    }
    if !shared.is_empty() {
        ops.push(shared_patch(id, shared, version));
    }
    ops
}

/// 部分更新仍保留空集no-op，不将缺省字段补成默认值。
pub fn update_channel_partial(
    channel_id: ChannelId,
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
) -> Option<Effect> {
    let ops = update_channel_partial_ops(channel_id, cols);
    (!ops.is_empty()).then_some(Effect::PersistFire { ops })
}

/// 共享属性按快照版本写；成员/消息派生列保持独立，不被共享水位拦截。
pub fn update_channel_partial_ops(
    channel_id: ChannelId,
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
) -> Vec<StorageOp> {
    use super::channel_snapshot::{is_shared, revision, shared_patch};
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let row: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let version = revision(&row);
    let (shared, patch): (Row, Row) = row.into_iter().partition(|(k, _)| is_shared(k));
    let id = SqlValue::Text(channel_id.as_str().to_string());
    let mut ops = Vec::with_capacity(2);
    if !patch.is_empty() {
        ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![id.clone()],
            patch,
        }));
    }
    if !shared.is_empty() {
        ops.push(shared_patch(id, shared, version));
    }
    ops
}

/// `update_channel` 后端 per-member 绝对态补丁 → `channel_member` 复合 PK upsert。
///
/// 这是「当前登录用户在某 channel 的 dialog badge 真值」，不是全群 `channel` 公共字段。
/// Go 后端会按可见性/mention/urgent 计算 member 维度计数后定向广播 `update_channel`，
/// Helix 只覆盖帧中出现的字段，不把 member 计数写回 channel 行。
pub fn upsert_channel_member_channel(row: Row) -> Option<Effect> {
    Some(Effect::PersistFire {
        ops: vec![upsert_channel_member_channel_op(row)?],
    })
}

/// 返回可并入相关事务的当前 viewer 对话绝对态 upsert。
pub fn upsert_channel_member_channel_op(row: Row) -> Option<StorageOp> {
    if row.is_empty() {
        return None;
    }
    Some(StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_member",
        rows: vec![row],
        conflict_key: Some("channel_id,user_id"),
        exclude_from_update: Vec::new(),
    }))
}

/// Canonical member projection persistence: create the composite-key row if needed,
/// then advance the revision, or atomically heal a same-revision row with an empty effect identity.
pub fn canonical_member_projection_ops(
    channel_id: ChannelId,
    user_id: &str,
    revision: u64,
    row: Row,
) -> Vec<StorageOp> {
    use helix_core::effect::{ScopedGuardedBumpSpec, SqlValue};

    let baseline = StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "channel_member",
        rows: vec![vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.as_str().to_string()),
            ),
            ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        ]],
        conflict_key: Some("channel_id,user_id"),
        exclude_from_update: vec!["channel_id", "user_id"],
    });
    // Only a complete authoritative row may heal the old partial-snapshot cache.
    let complete = [
        "effect_id",
        "projection_revision",
        "unread_count",
        "reaction_unread_count",
        "unread_post_id",
        "last_post",
        "last_read_seq",
        "mention_list",
        "mention_count",
        "mention_count_root",
        "mention_user",
        "urgent_mention_count",
        "urgent_post_list",
        "urgent_mention_user",
        "msg_count",
        "msg_count_root",
        "msg_count_private",
        "last_post_at",
        "last_root_post_at",
    ]
    .iter()
    .all(|required| {
        row.iter().any(|(column, value)| {
            column == required
                && match value {
                    SqlValue::Text(value) => *required != "effect_id" || !value.trim().is_empty(),
                    SqlValue::Integer(value) => *value >= 0,
                    _ => false,
                }
        })
    });
    let set_cols = row
        .into_iter()
        .filter(|(column, _)| !matches!(column.as_str(), "channel_id" | "user_id" | "updated_at"))
        .collect();
    let advance = ScopedGuardedBumpSpec {
        comparison: helix_core::effect::GuardComparison::Greater,
        table: "channel_member",
        scope_col: "channel_id",
        scope_val: SqlValue::Text(channel_id.as_str().to_string()),
        key_col: "user_id",
        key_val: SqlValue::Text(user_id.to_string()),
        bump_col: "updated_at",
        bump_delta: 0,
        set_cols,
        guard_col: "projection_revision",
        guard_val: revision.min(i64::MAX as u64) as i64,
        additional_guard: None,
    };
    let mut ops = vec![baseline];
    if complete && revision > 0 && revision <= i64::MAX as u64 {
        let mut repair = advance.clone();
        repair.comparison = helix_core::effect::GuardComparison::Equal;
        repair.additional_guard = Some(("effect_id", SqlValue::Text(String::new())));
        ops.push(StorageOp::ScopedGuardedBump(repair));
    }
    ops.push(StorageOp::ScopedGuardedBump(advance));
    ops
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
        // Dialog scans order by last_post_at. Advancing only the guard column
        // leaves a recovered channel outside the bounded first window even
        // though its durable last_post is already newer.
        set_cols.push((
            "last_post_at".to_string(),
            SqlValue::Integer(upd.msg_create_at),
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
/// 由 driver 顺序执行并把最后一个 `Get` 的 row 回给 Helix 组装累计 ChannelUpdate。
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
        version_column: None,
        update_guard: None,
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

/// 关闭事实没有时间语义；与 terminal cursor 同事务保存独立布尔投影。
pub fn closed_channel_op(channel_id: ChannelId) -> StorageOp {
    crate::acl::to_effect_s1::channel_set_cols_op(
        channel_id,
        vec![("is_closed", helix_core::effect::SqlValue::Integer(1))],
    )
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
            ("mention_list".to_string(), SqlValue::Text("[]".into())),
            ("mention_user".to_string(), SqlValue::Text(String::new())),
            ("urgent_count".to_string(), SqlValue::Integer(0)),
            ("urgent_post_list".to_string(), SqlValue::Text("[]".into())),
            ("has_urgent_post".to_string(), SqlValue::Integer(0)),
            ("has_more".to_string(), SqlValue::Integer(1)),
        ],
    })
}
