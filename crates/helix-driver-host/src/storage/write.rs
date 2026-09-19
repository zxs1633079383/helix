use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GuardedBumpSpec, MonotonicUpsertSpec, Row,
    ScopedGuardedBumpSpec, SqlValue, StorageOp, UpsertGuard, UpsertSpec,
};
use helix_core::PortError;
use rusqlite::params_from_iter;
use rusqlite::types::{Value, ValueRef};
use rusqlite::Transaction;
use sha2::{Digest, Sha256};

use super::connection::{map_join_err, map_lock_err, map_sqlite_err};
use super::trace::{
    current_storage_operation_context, current_storage_trace_context, offline_sync_trace_enabled,
    offline_sync_wal_enabled, trace_sql, wal_snapshot, StorageOperationContext,
    StorageTraceContext,
};
use super::values::sql_values;
use super::HostStorage;

/// 校验 guarded upsert 的单行/冲突键前置条件，避免批量规格产生不明确的部分成功。
fn validate_guarded_upsert(spec: &UpsertSpec) -> Result<(), PortError> {
    if let Some(column) = spec.version_column {
        if spec.conflict_key.is_none()
            || spec.rows.iter().any(|row| {
                !row.iter()
                    .any(|(key, value)| key == column && !matches!(value, SqlValue::Null))
            })
        {
            return Err(PortError::Storage(
                "versioned upsert requires a conflict key and non-null version in every row".into(),
            ));
        }
    }
    if spec.update_guard.is_none() {
        return Ok(());
    }
    if spec.conflict_key.is_none() {
        return Err(PortError::Storage(
            "UpsertGuard requires a conflict key".to_string(),
        ));
    }
    if spec.rows.len() != 1 {
        return Err(PortError::Storage(
            "UpsertGuard accepts exactly one row".to_string(),
        ));
    }
    Ok(())
}

/// 将 upsert 行和可选守卫值按 SQL 占位符顺序编码；守卫值始终走绑定参数。
fn upsert_values(row: &Row, guard: Option<&UpsertGuard>) -> Vec<Value> {
    let mut values = sql_values(row.iter().map(|(_, value)| value));
    if let Some(guard) = guard {
        values.extend(sql_values(std::iter::once(&guard.expected)));
    }
    values
}

/// 将冲突行守卫失败转换为可由 PersistAtomic 识别并触发回滚的存储错误。
fn guarded_upsert_mismatch(_table: &str, _column: &str) -> PortError {
    PortError::StorageConflict
}

/// Execute row upserts on the blocking writer; a failed guard aborts the transaction.
pub(super) async fn batch_upsert(storage: &HostStorage, spec: UpsertSpec) -> Result<(), PortError> {
    validate_guarded_upsert(&spec)?;
    if spec.rows.is_empty() {
        return Ok(());
    }
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    tokio::task::spawn_blocking(move || {
        let mut conn = writer.lock().map_err(map_lock_err)?;
        let sql = crate::upsert_sql::upsert_sql_for_shape(&spec);
        let tx = conn.transaction().map_err(map_sqlite_err)?;
        {
            let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
            for row in &spec.rows {
                let values = upsert_values(row, spec.update_guard.as_ref());
                let _span = trace_sql(&trace, "UPSERT", Some(spec.table), &sql, &values);
                let affected = stmt
                    .execute(params_from_iter(values.iter()))
                    .map_err(map_sqlite_err)?;
                if affected == 0 {
                    let Some(guard) = spec.update_guard.as_ref() else {
                        continue;
                    };
                    return Err(guarded_upsert_mismatch(spec.table, guard.column));
                }
            }
        }
        tx.commit().map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

pub(super) async fn batch_update(
    storage: &HostStorage,
    spec: BatchUpdateSpec,
) -> Result<(), PortError> {
    let BatchUpdateSpec {
        table,
        key_col,
        key_vals,
        patch,
    } = spec;
    if key_vals.is_empty() || patch.is_empty() {
        return Ok(());
    }
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    let table = table.to_string();
    let key_col = key_col.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = writer.lock().map_err(map_lock_err)?;
        let set_clause: Vec<String> = patch.iter().map(|(k, _)| format!("{k} = ?")).collect();
        let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();
        let sql = format!(
            "UPDATE {table} SET {set} WHERE {key_col} IN ({in_vals})",
            set = set_clause.join(", "),
            in_vals = in_placeholders.join(", "),
        );
        let mut values = sql_values(patch.iter().map(|(_, value)| value));
        values.extend(sql_values(key_vals.iter()));
        let _span = trace_sql(&trace, "UPDATE", Some(&table), &sql, &values);
        conn.execute(&sql, params_from_iter(values.iter()))
            .map(|_| ())
            .map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

pub(super) async fn monotonic_upsert(
    storage: &HostStorage,
    spec: MonotonicUpsertSpec,
) -> Result<(), PortError> {
    const CURSOR_ADVANCE_UPSERT_SQL: &str =
        "INSERT INTO channel_event_cursor (channel_id, last_event_seq, updated_at) \
         VALUES (?, ?, ?) ON CONFLICT(channel_id) DO UPDATE SET \
         last_event_seq = MAX(channel_event_cursor.last_event_seq, excluded.last_event_seq), \
         updated_at = excluded.updated_at";
    let sql: Cow<'static, str> = match (spec.table, spec.key_col, spec.value_col, spec.touch_col) {
        ("channel_event_cursor", "channel_id", "last_event_seq", Some("updated_at")) => {
            Cow::Borrowed(CURSOR_ADVANCE_UPSERT_SQL)
        }
        (table, key, val, Some(touch)) => Cow::Owned(format!(
            "INSERT INTO {table} ({key}, {val}, {touch}) VALUES (?, ?, ?) \
             ON CONFLICT({key}) DO UPDATE SET \
             {val} = MAX({table}.{val}, excluded.{val}), {touch} = excluded.{touch}",
        )),
        (table, key, val, None) => Cow::Owned(format!(
            "INSERT INTO {table} ({key}, {val}) VALUES (?, ?) \
             ON CONFLICT({key}) DO UPDATE SET {val} = excluded.{val} \
             WHERE excluded.{val} > {table}.{val}",
        )),
    };
    let now_ms = match spec.touch_col {
        Some(_) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        None => 0,
    };
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    tokio::task::spawn_blocking(move || {
        let conn = writer.lock().map_err(map_lock_err)?;
        let mut stmt = conn.prepare_cached(sql.as_ref()).map_err(map_sqlite_err)?;
        let params = match spec.touch_col {
            Some(_) => vec![
                Value::Text(spec.scope_key.clone()),
                Value::Integer(spec.value),
                Value::Integer(now_ms),
            ],
            None => vec![
                Value::Text(spec.scope_key.clone()),
                Value::Integer(spec.value),
            ],
        };
        let _span = trace_sql(&trace, "UPSERT", Some(spec.table), sql.as_ref(), &params);
        stmt.execute(params_from_iter(params.iter()))
            .map(|_| ())
            .map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

pub(super) async fn guarded_bump(
    storage: &HostStorage,
    spec: GuardedBumpSpec,
) -> Result<(), PortError> {
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    tokio::task::spawn_blocking(move || {
        let mut set_parts = vec![format!("{bump} = {bump} + ?", bump = spec.bump_col)];
        set_parts.extend(
            spec.extra_bumps
                .iter()
                .map(|(bump, _)| format!("{bump} = {bump} + ?")),
        );
        set_parts.extend(spec.set_cols.iter().map(|(k, _)| format!("{k} = ?")));
        let sql = format!(
            "UPDATE {table} SET {set} WHERE {key} = ? AND ? > {guard}",
            table = spec.table,
            set = set_parts.join(", "),
            key = spec.key_col,
            guard = spec.guard_col,
        );
        let mut params = sql_values(std::iter::once(&SqlValue::Integer(spec.bump_delta)));
        params.extend(
            spec.extra_bumps
                .iter()
                .map(|(_, delta)| Value::Integer(*delta)),
        );
        params.extend(sql_values(spec.set_cols.iter().map(|(_, value)| value)));
        params.extend(sql_values(std::iter::once(&spec.key_val)));
        params.extend(sql_values(std::iter::once(&SqlValue::Integer(
            spec.guard_val,
        ))));
        let conn = writer.lock().map_err(map_lock_err)?;
        let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
        let _span = trace_sql(&trace, "UPDATE", Some(spec.table), &sql, &params);
        stmt.execute(params_from_iter(params.iter()))
            .map(|_| ())
            .map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

/// 在复合 scope + key 上以单条 SQL 执行守卫式自增与绝对列 patch。
pub(super) async fn scoped_guarded_bump(
    storage: &HostStorage,
    spec: ScopedGuardedBumpSpec,
) -> Result<(), PortError> {
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    tokio::task::spawn_blocking(move || {
        let mut set_parts = vec![format!("{bump} = {bump} + ?", bump = spec.bump_col)];
        set_parts.extend(
            spec.set_cols
                .iter()
                .map(|(column, _)| format!("{column} = ?")),
        );
        let sql = format!(
            "UPDATE {table} SET {set} WHERE {scope_col} = ? AND {key_col} = ? AND ? > {guard}",
            table = spec.table,
            set = set_parts.join(", "),
            scope_col = spec.scope_col,
            key_col = spec.key_col,
            guard = spec.guard_col,
        );
        let mut params = sql_values(std::iter::once(&SqlValue::Integer(spec.bump_delta)));
        params.extend(sql_values(spec.set_cols.iter().map(|(_, value)| value)));
        params.extend(sql_values(std::iter::once(&spec.scope_val)));
        params.extend(sql_values(std::iter::once(&spec.key_val)));
        params.extend(sql_values(std::iter::once(&SqlValue::Integer(
            spec.guard_val,
        ))));
        let conn = writer.lock().map_err(map_lock_err)?;
        let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
        let _span = trace_sql(&trace, "UPDATE", Some(spec.table), &sql, &params);
        stmt.execute(params_from_iter(params.iter()))
            .map(|_| ())
            .map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

pub(super) async fn batch_delete(
    storage: &HostStorage,
    spec: BatchDeleteSpec,
) -> Result<(), PortError> {
    let BatchDeleteSpec {
        table,
        scope_col,
        scope_val,
        key_col,
        key_vals,
    } = spec;
    if key_vals.is_empty() {
        return Ok(());
    }
    let writer = Arc::clone(&storage.writer);
    let trace = current_storage_trace_context();
    let table = table.to_string();
    let scope_col = scope_col.to_string();
    let key_col = key_col.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = writer.lock().map_err(map_lock_err)?;
        let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();
        let sql = format!(
            "DELETE FROM {table} WHERE {scope_col} = ? AND {key_col} IN ({in_vals})",
            in_vals = in_placeholders.join(", "),
        );
        let mut values = sql_values(std::iter::once(&scope_val));
        values.extend(sql_values(key_vals.iter()));
        let _span = trace_sql(&trace, "DELETE", Some(&table), &sql, &values);
        conn.execute(&sql, params_from_iter(values.iter()))
            .map(|_| ())
            .map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

/// Execute write-only `StorageOp`s in one SQLite transaction.
///
/// This intentionally does not reuse the public `Storage` methods: each of those owns its own
/// transaction/connection boundary, which is correct for legacy `Persist` but would silently
/// break `PersistAtomic`. Read operations are rejected so the correlated reply remains an
/// unambiguous commit acknowledgement rather than a mixed read/write protocol.
pub(super) async fn atomic_write(
    storage: &HostStorage,
    ops: Vec<StorageOp>,
) -> Result<(), PortError> {
    let writer = Arc::clone(&storage.writer);
    let db_target = Arc::clone(&storage.db_target);
    let trace = current_storage_trace_context();
    let operation_context = current_storage_operation_context();
    tokio::task::spawn_blocking(move || {
        let mut conn = writer.lock().map_err(map_lock_err)?;
        let trace_enabled = offline_sync_trace_enabled();
        let wal_enabled = offline_sync_wal_enabled();
        if wal_enabled {
            tracing::info!(
                target: "offline_sync",
                hop = "offline_sync.wal",
                phase = "before_write",
                wal = %wal_snapshot(db_target.as_str()),
                "SQLite WAL 元数据（写入前）"
            );
        }
        let tx = conn.transaction().map_err(map_sqlite_err)?;
        let mut affected_rows = Vec::with_capacity(ops.len());
        let mut partial = false;
        for (index, op) in ops.iter().enumerate() {
            let rows = match atomic_write_op(&tx, op, &trace) {
                Ok(rows) => rows,
                Err(error) => {
                    if trace_enabled {
                        log_atomic_failure(&operation_context, index, op, &ops, &error);
                    }
                    return Err(error);
                }
            };
            if rows == 0 {
                partial = true;
                if trace_enabled {
                    log_zero_affected_rows(&operation_context, index, op, &ops);
                }
                if is_fatal_zero_row(op) {
                    let error = PortError::Storage(format!(
                        "sync type=2 message update matched 0 rows (operation_index={index})"
                    ));
                    if trace_enabled {
                        log_atomic_failure(&operation_context, index, op, &ops, &error);
                    }
                    return Err(error);
                }
            }
            if trace_enabled {
                log_atomic_write(&operation_context, index, op, &ops, rows);
            }
            affected_rows.push(rows);
        }
        tx.commit().map_err(map_sqlite_err)?;
        if trace_enabled {
            log_atomic_summary(&operation_context, &ops, &affected_rows, partial, true);
        }
        if wal_enabled {
            tracing::info!(
                target: "offline_sync",
                hop = "offline_sync.wal",
                phase = "after_commit",
                wal = %wal_snapshot(db_target.as_str()),
                "SQLite WAL 元数据（提交后）"
            );
        }
        if trace_enabled {
            read_back_messages(&conn, &operation_context, &ops);
        }
        if wal_enabled {
            tracing::info!(
                target: "offline_sync",
                hop = "offline_sync.wal",
                phase = "after_readback",
                wal = %wal_snapshot(db_target.as_str()),
                "SQLite WAL 元数据（readback 后）"
            );
        }
        Ok(())
    })
    .await
    .map_err(map_join_err)?
}

fn atomic_write_op(
    tx: &Transaction<'_>,
    op: &StorageOp,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    match op {
        StorageOp::BatchUpsert(spec) => atomic_batch_upsert(tx, spec, trace),
        StorageOp::MonotonicUpsert(spec) => atomic_monotonic_upsert(tx, spec, trace),
        StorageOp::BatchUpdate(spec) => atomic_batch_update(tx, spec, trace),
        StorageOp::GuardedBump(spec) => atomic_guarded_bump(tx, spec, trace),
        StorageOp::ScopedGuardedBump(spec) => atomic_scoped_guarded_bump(tx, spec, trace),
        StorageOp::BatchDelete(spec) => atomic_batch_delete(tx, spec, trace),
        StorageOp::Get(_)
        | StorageOp::ScopedGet(_)
        | StorageOp::ScopedMax(_)
        | StorageOp::ScopedScan(_)
        | StorageOp::Scan(_) => Err(PortError::Storage(
            "atomic storage write does not permit read operations".to_string(),
        )),
    }
}

/// Apply upserts inside the caller transaction; guard misses roll back the complete batch.
fn atomic_batch_upsert(
    tx: &Transaction<'_>,
    spec: &UpsertSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    validate_guarded_upsert(spec)?;
    if spec.rows.is_empty() {
        return Ok(0);
    }
    let sql = crate::upsert_sql::upsert_sql_for_shape(spec);
    let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
    let mut affected_rows = 0;
    for row in &spec.rows {
        let values = upsert_values(row, spec.update_guard.as_ref());
        let _span = trace_sql(trace, "UPSERT", Some(spec.table), &sql, &values);
        let affected = stmt
            .execute(params_from_iter(values.iter()))
            .map_err(map_sqlite_err)?;
        if affected == 0 {
            let Some(guard) = spec.update_guard.as_ref() else {
                affected_rows += affected;
                continue;
            };
            return Err(guarded_upsert_mismatch(spec.table, guard.column));
        }
        affected_rows += affected;
    }
    Ok(affected_rows)
}

fn atomic_monotonic_upsert(
    tx: &Transaction<'_>,
    spec: &MonotonicUpsertSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    const CURSOR_ADVANCE_UPSERT_SQL: &str =
        "INSERT INTO channel_event_cursor (channel_id, last_event_seq, updated_at) \
         VALUES (?, ?, ?) ON CONFLICT(channel_id) DO UPDATE SET \
         last_event_seq = MAX(channel_event_cursor.last_event_seq, excluded.last_event_seq), \
         updated_at = excluded.updated_at";
    let sql: Cow<'static, str> = match (spec.table, spec.key_col, spec.value_col, spec.touch_col) {
        ("channel_event_cursor", "channel_id", "last_event_seq", Some("updated_at")) => {
            Cow::Borrowed(CURSOR_ADVANCE_UPSERT_SQL)
        }
        (table, key, val, Some(touch)) => Cow::Owned(format!(
            "INSERT INTO {table} ({key}, {val}, {touch}) VALUES (?, ?, ?) \
             ON CONFLICT({key}) DO UPDATE SET \
             {val} = MAX({table}.{val}, excluded.{val}), {touch} = excluded.{touch}",
        )),
        (table, key, val, None) => Cow::Owned(format!(
            "INSERT INTO {table} ({key}, {val}) VALUES (?, ?) \
             ON CONFLICT({key}) DO UPDATE SET {val} = excluded.{val} \
             WHERE excluded.{val} > {table}.{val}",
        )),
    };
    let now_ms = spec
        .touch_col
        .map(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0)
        })
        .unwrap_or_default();
    let params = if spec.touch_col.is_some() {
        vec![
            Value::Text(spec.scope_key.clone()),
            Value::Integer(spec.value),
            Value::Integer(now_ms),
        ]
    } else {
        vec![
            Value::Text(spec.scope_key.clone()),
            Value::Integer(spec.value),
        ]
    };
    let _span = trace_sql(trace, "UPSERT", Some(spec.table), sql.as_ref(), &params);
    tx.execute(sql.as_ref(), params_from_iter(params.iter()))
        .map_err(map_sqlite_err)
}

fn atomic_batch_update(
    tx: &Transaction<'_>,
    spec: &BatchUpdateSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    if spec.key_vals.is_empty() || spec.patch.is_empty() {
        return Ok(0);
    }
    let set_clause: Vec<String> = spec
        .patch
        .iter()
        .map(|(key, _)| format!("{key} = ?"))
        .collect();
    let placeholders: Vec<&str> = spec.key_vals.iter().map(|_| "?").collect();
    let sql = format!(
        "UPDATE {table} SET {set_clause} WHERE {key_col} IN ({placeholders})",
        table = spec.table,
        set_clause = set_clause.join(", "),
        key_col = spec.key_col,
        placeholders = placeholders.join(", "),
    );
    let mut values = sql_values(spec.patch.iter().map(|(_, value)| value));
    values.extend(sql_values(spec.key_vals.iter()));
    let _span = trace_sql(trace, "UPDATE", Some(spec.table), &sql, &values);
    tx.execute(&sql, params_from_iter(values.iter()))
        .map_err(map_sqlite_err)
}

fn atomic_guarded_bump(
    tx: &Transaction<'_>,
    spec: &GuardedBumpSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    let mut set_parts = vec![format!("{} = {} + ?", spec.bump_col, spec.bump_col)];
    set_parts.extend(
        spec.extra_bumps
            .iter()
            .map(|(column, _)| format!("{column} = {column} + ?")),
    );
    set_parts.extend(
        spec.set_cols
            .iter()
            .map(|(column, _)| format!("{column} = ?")),
    );
    let sql = format!(
        "UPDATE {table} SET {set_clause} WHERE {key_col} = ? AND ? > {guard_col}",
        table = spec.table,
        set_clause = set_parts.join(", "),
        key_col = spec.key_col,
        guard_col = spec.guard_col,
    );
    let mut params = sql_values(std::iter::once(&SqlValue::Integer(spec.bump_delta)));
    params.extend(
        spec.extra_bumps
            .iter()
            .map(|(_, delta)| Value::Integer(*delta)),
    );
    params.extend(sql_values(spec.set_cols.iter().map(|(_, value)| value)));
    params.extend(sql_values(std::iter::once(&spec.key_val)));
    params.extend(sql_values(std::iter::once(&SqlValue::Integer(
        spec.guard_val,
    ))));
    let _span = trace_sql(trace, "UPDATE", Some(spec.table), &sql, &params);
    tx.execute(&sql, params_from_iter(params.iter()))
        .map_err(map_sqlite_err)
}

/// 在既有事务内兑现复合作用域守卫更新，保持 PersistAtomic 全写回滚语义。
fn atomic_scoped_guarded_bump(
    tx: &Transaction<'_>,
    spec: &ScopedGuardedBumpSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    let mut set_parts = vec![format!("{bump} = {bump} + ?", bump = spec.bump_col)];
    set_parts.extend(
        spec.set_cols
            .iter()
            .map(|(column, _)| format!("{column} = ?")),
    );
    let sql = format!(
        "UPDATE {table} SET {set} WHERE {scope_col} = ? AND {key_col} = ? AND ? > {guard}",
        table = spec.table,
        set = set_parts.join(", "),
        scope_col = spec.scope_col,
        key_col = spec.key_col,
        guard = spec.guard_col,
    );
    let mut params = sql_values(std::iter::once(&SqlValue::Integer(spec.bump_delta)));
    params.extend(sql_values(spec.set_cols.iter().map(|(_, value)| value)));
    params.extend(sql_values(std::iter::once(&spec.scope_val)));
    params.extend(sql_values(std::iter::once(&spec.key_val)));
    params.extend(sql_values(std::iter::once(&SqlValue::Integer(
        spec.guard_val,
    ))));
    let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
    let _span = trace_sql(trace, "UPDATE", Some(spec.table), &sql, &params);
    stmt.execute(params_from_iter(params.iter()))
        .map_err(map_sqlite_err)
}

fn atomic_batch_delete(
    tx: &Transaction<'_>,
    spec: &BatchDeleteSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<usize, PortError> {
    if spec.key_vals.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<&str> = spec.key_vals.iter().map(|_| "?").collect();
    let sql = format!(
        "DELETE FROM {table} WHERE {scope_col} = ? AND {key_col} IN ({placeholders})",
        table = spec.table,
        scope_col = spec.scope_col,
        key_col = spec.key_col,
        placeholders = placeholders.join(", "),
    );
    let mut values = sql_values(std::iter::once(&spec.scope_val));
    values.extend(sql_values(spec.key_vals.iter()));
    let _span = trace_sql(trace, "DELETE", Some(spec.table), &sql, &values);
    tx.execute(&sql, params_from_iter(values.iter()))
        .map_err(map_sqlite_err)
}

/// 记录单个 StorageOp 的实际 SQL 影响行数和脱敏形状。
fn log_atomic_write(
    context: &Option<StorageOperationContext>,
    index: usize,
    op: &StorageOp,
    ops: &[StorageOp],
    affected_rows: usize,
) {
    if !storage_target_allowed(op, ops) {
        return;
    }
    let (operation, table) = op_identity(op);
    let msg_id = storage_key_hint(op);
    let temporary_id = storage_temporary_id_hint(op, ops);
    tracing::info!(target: "offline_sync",
        hop = "sync.sqlite.write",
        corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        upstream_corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        track_id = context
            .as_ref()
            .and_then(|value| value.track_id.as_deref())
            .unwrap_or("helix-storage"),
        channel_id = storage_channel_id_hint(op, ops),
        event_seq = storage_event_seq_hint(op, ops),
        event_type = storage_event_type(op),
        msg_id = msg_id.as_str(),
        temporary_id = temporary_id.as_str(),
        source = "helix-driver-host",
        operation_id = format!("storage-op:{index}"),
        operation,
        table,
        affected_rows,
        "SQLite 写操作完成"
    );
}

/// 记录 SQL 错误并保留事务回滚边界。
fn log_atomic_failure(
    context: &Option<StorageOperationContext>,
    index: usize,
    op: &StorageOp,
    ops: &[StorageOp],
    error: &PortError,
) {
    if !storage_target_allowed(op, ops) {
        return;
    }
    let (operation, table) = op_identity(op);
    let msg_id = storage_key_hint(op);
    let temporary_id = storage_temporary_id_hint(op, ops);
    tracing::error!(target: "offline_sync",
        hop = "sync.sqlite.write_failed",
        corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        upstream_corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        track_id = context
            .as_ref()
            .and_then(|value| value.track_id.as_deref())
            .unwrap_or("helix-storage"),
        channel_id = storage_channel_id_hint(op, ops),
        event_seq = storage_event_seq_hint(op, ops),
        event_type = storage_event_type(op),
        msg_id = msg_id.as_str(),
        temporary_id = temporary_id.as_str(),
        source = "helix-driver-host",
        operation_id = format!("storage-op:{index}"),
        operation,
        table,
        affected_rows = 0_usize,
        error = ?error,
        "SQLite 写操作失败，PersistAtomic 将回滚"
    );
}

/// 记录 0 行命中；type=2 消息更新由调用方随后 fail-closed，其它 phantom/no-op 保留旧语义但标记 partial。
fn log_zero_affected_rows(
    context: &Option<StorageOperationContext>,
    index: usize,
    op: &StorageOp,
    ops: &[StorageOp],
) {
    if !storage_target_allowed(op, ops) {
        return;
    }
    let cursor_target = ops.iter().find_map(|candidate| match candidate {
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor" => {
            Some(spec.value)
        }
        _ => None,
    });
    let (operation, table) = op_identity(op);
    let event_type = storage_event_type(op);
    let msg_id = storage_key_hint(op);
    let temporary_id = storage_temporary_id_hint(op, ops);
    tracing::warn!(target: "offline_sync",
        hop = "sync.sqlite.write_zero_rows",
        corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        upstream_corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        track_id = context
            .as_ref()
            .and_then(|value| value.track_id.as_deref())
            .unwrap_or("helix-storage"),
        channel_id = storage_channel_id_hint(op, ops),
        event_seq = storage_event_seq_hint(op, ops),
        event_type,
        msg_id = msg_id.as_str(),
        temporary_id = temporary_id.as_str(),
        source = "helix-driver-host",
        operation_id = format!("storage-op:{index}"),
        operation,
        table,
        affected_rows = 0_usize,
        cursor_target = cursor_target.unwrap_or_default(),
        partial = true,
        "SQLite 写操作未命中目标行，事务结果标记为 partial"
    );
    if is_fatal_zero_row(op) {
        tracing::error!(target: "offline_sync",
            hop = "sync.type2.zero_rows",
            corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
            upstream_corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
            track_id = context
                .as_ref()
                .and_then(|value| value.track_id.as_deref())
                .unwrap_or("helix-storage"),
            channel_id = storage_channel_id_hint(op, ops),
            event_seq = storage_event_seq_hint(op, ops),
            event_type = 2_u8,
            msg_id = msg_id.as_str(),
            temporary_id = temporary_id.as_str(),
            source = "helix-driver-host",
            operation_id = format!("storage-op:{index}"),
            cursor_target = cursor_target.unwrap_or_default(),
            "type=2 未命中消息行，禁止推进同步游标，PersistAtomic fail-closed"
        );
    }
}

/// 记录 PersistAtomic 的提交摘要，包含按操作形状归类的类型和影响行数。
fn log_atomic_summary(
    context: &Option<StorageOperationContext>,
    ops: &[StorageOp],
    affected_rows: &[usize],
    partial: bool,
    commit: bool,
) {
    let mut type_counts = HashMap::<&str, usize>::new();
    let mut affected_by_type = HashMap::<&str, usize>::new();
    for (op, rows) in ops.iter().zip(affected_rows.iter().copied()) {
        let kind = storage_event_type(op);
        *type_counts.entry(kind).or_default() += 1;
        *affected_by_type.entry(kind).or_default() += rows;
    }
    let cursor_target = ops.iter().find_map(|op| match op {
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor" => {
            Some(spec.value)
        }
        _ => None,
    });
    tracing::info!(target: "offline_sync",
        hop = "sync.persist.summary",
        phase = "after_commit",
        corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        upstream_corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0),
        track_id = context
            .as_ref()
            .and_then(|value| value.track_id.as_deref())
            .unwrap_or("helix-storage"),
        channel_id = ops
            .iter()
            .map(|op| storage_channel_id_hint(op, ops))
            .find(|channel_id| !channel_id.is_empty())
            .unwrap_or_default(),
        event_seq = cursor_target.unwrap_or_default(),
        event_type = "batch",
        msg_id = "",
        source = "helix-driver-host",
        operation_id = format!(
            "sync-persist:{}",
            context.as_ref().and_then(|value| value.corr).unwrap_or(0)
        ),
        operation_count = ops.len(),
        type_counts = ?type_counts,
        affected_rows_by_type = ?affected_by_type,
        cursor_target = cursor_target.unwrap_or_default(),
        partial,
        commit,
        "同步批次事务提交结果"
    );
}

/// type=2 必须命中已有 message 行，否则回滚整批并阻止 cursor 前进；type3/type6 phantom 继续显式告警。
fn is_fatal_zero_row(op: &StorageOp) -> bool {
    matches!(
        op,
        StorageOp::BatchUpdate(spec)
            if spec.table == "message"
                && spec.patch.iter().any(|(column, _)| column == "type")
                && spec.patch.iter().any(|(column, _)| column == "message")
                && spec.patch.iter().any(|(column, _)| column == "props")
    )
}

/// 从 StorageOp 形状返回可关联的操作名称与表名。
fn op_identity(op: &StorageOp) -> (&'static str, &'static str) {
    match op {
        StorageOp::BatchUpsert(spec) => ("BatchUpsert", spec.table),
        StorageOp::MonotonicUpsert(spec) => ("MonotonicUpsert", spec.table),
        StorageOp::BatchUpdate(spec) => ("BatchUpdate", spec.table),
        StorageOp::GuardedBump(spec) => ("GuardedBump", spec.table),
        StorageOp::ScopedGuardedBump(spec) => ("ScopedGuardedBump", spec.table),
        StorageOp::Get(spec) => ("Get", spec.table),
        StorageOp::ScopedGet(spec) => ("ScopedGet", spec.table),
        StorageOp::ScopedMax(spec) => ("ScopedMax", spec.table),
        StorageOp::ScopedScan(spec) => ("ScopedScan", spec.table),
        StorageOp::Scan(spec) => ("Scan", spec.table),
        StorageOp::BatchDelete(spec) => ("BatchDelete", spec.table),
    }
}

/// 根据实际 SQL patch 形状归类 type1/type2/type3/type6，其它写入归为 other。
fn storage_event_type(op: &StorageOp) -> &'static str {
    match op {
        StorageOp::BatchUpsert(spec) if spec.table == "message" => "type1",
        StorageOp::BatchUpdate(spec)
            if spec.table == "message"
                && spec.patch.iter().any(|(column, _)| column == "revoke") =>
        {
            "type3"
        }
        StorageOp::BatchUpdate(spec)
            if spec.table == "message"
                && spec.patch.iter().any(|(column, _)| column == "read_bits") =>
        {
            "type6"
        }
        StorageOp::BatchUpdate(spec) if spec.table == "message" => "type2",
        _ => "other",
    }
}

/// 从当前批次的实际 StorageOp 推导频道作用域，避免 driver 日志退化成无意义的占位值。
fn storage_channel_id_hint(op: &StorageOp, ops: &[StorageOp]) -> String {
    if let Some(channel_id) = direct_channel_id(op) {
        return channel_id;
    }
    ops.iter().find_map(direct_channel_id).unwrap_or_default()
}

/// 读取操作自身携带的频道键；消息 type=2/3/6 再由同批 type=1 行补齐作用域。
fn direct_channel_id(op: &StorageOp) -> Option<String> {
    fn text(value: &SqlValue) -> Option<String> {
        match value {
            SqlValue::Text(value) if !value.is_empty() => Some(value.clone()),
            _ => None,
        }
    }
    fn row_value<'a>(row: &'a [(String, SqlValue)], column: &str) -> Option<&'a SqlValue> {
        row.iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    match op {
        StorageOp::BatchUpsert(spec) if spec.table == "message" => spec
            .rows
            .first()
            .and_then(|row| row_value(row, "channel_id"))
            .and_then(text),
        StorageOp::BatchUpdate(spec) if spec.table == "channel" => {
            spec.key_vals.first().and_then(text)
        }
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor" => {
            Some(spec.scope_key.clone())
        }
        StorageOp::GuardedBump(spec) if spec.table == "channel" => text(&spec.key_val),
        StorageOp::ScopedGuardedBump(spec) => text(&spec.scope_val),
        StorageOp::BatchDelete(spec) => text(&spec.scope_val),
        _ => None,
    }
}

/// 从消息 upsert 或 cursor 操作提取可关联的事件序号；没有该列时保留 0。
fn storage_event_seq_hint(op: &StorageOp, ops: &[StorageOp]) -> u64 {
    fn integer(value: &SqlValue) -> Option<u64> {
        match value {
            SqlValue::Integer(value) if *value >= 0 => Some(*value as u64),
            _ => None,
        }
    }
    fn row_value<'a>(row: &'a [(String, SqlValue)], column: &str) -> Option<&'a SqlValue> {
        row.iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    match op {
        StorageOp::BatchUpsert(spec) if spec.table == "message" => spec
            .rows
            .first()
            .and_then(|row| row_value(row, "event_seq"))
            .and_then(integer)
            .unwrap_or_default(),
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor" => {
            spec.value.max(0) as u64
        }
        _ => ops
            .iter()
            .find_map(|candidate| match candidate {
                StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor" => {
                    Some(spec.value.max(0) as u64)
                }
                _ => None,
            })
            .unwrap_or_default(),
    }
}

/// 提供脱敏的消息主键提示；不输出正文或用户快照。
fn storage_key_hint(op: &StorageOp) -> String {
    let key = match op {
        StorageOp::BatchUpdate(spec) => spec.key_vals.first(),
        StorageOp::BatchUpsert(spec) => spec
            .rows
            .first()
            .and_then(|row| row.iter().find(|(column, _)| column == "id"))
            .map(|(_, value)| value),
        _ => None,
    };
    match key {
        Some(SqlValue::Text(value)) => value.clone(),
        Some(SqlValue::Integer(value)) => value.to_string(),
        _ => String::new(),
    }
}

/// 从 message StorageOp 提取 temporary_id，修复 BatchUpsert 只记录 id 的关联缺口。
fn storage_temporary_id_hint(op: &StorageOp, ops: &[StorageOp]) -> String {
    fn text(value: &SqlValue) -> Option<String> {
        match value {
            SqlValue::Text(value) if !value.is_empty() => Some(value.clone()),
            _ => None,
        }
    }
    fn row_value<'a>(row: &'a [(String, SqlValue)], column: &str) -> Option<&'a SqlValue> {
        row.iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    match op {
        StorageOp::BatchUpsert(spec) if spec.table == "message" => spec
            .rows
            .first()
            .and_then(|row| row_value(row, "temporary_id"))
            .and_then(text)
            .unwrap_or_default(),
        StorageOp::BatchUpdate(spec) if spec.table == "message" => spec
            .key_vals
            .first()
            .and_then(|key| {
                ops.iter().find_map(|candidate| match candidate {
                    StorageOp::BatchUpsert(upsert) if upsert.table == "message" => upsert
                        .rows
                        .iter()
                        .find(|row| {
                            row_value(row, "id").map(storage_value_hint)
                                == Some(storage_value_hint(key))
                        })
                        .and_then(|row| row_value(row, "temporary_id"))
                        .and_then(text),
                    _ => None,
                })
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// 判断 message 诊断是否命中 HELIX_OFFLINE_SYNC_TARGETS。
fn storage_target_enabled(msg_id: &str, temporary_id: &str) -> bool {
    let targets = std::env::var("HELIX_OFFLINE_SYNC_TARGETS").unwrap_or_default();
    if targets.trim().is_empty() {
        return false;
    }
    targets
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .any(|target| {
            let aliases: &[&str] = match target {
                "qem" => &["qem", "helix_tmp_0000019fd1170a44_000000000000004f"],
                "bzj" => &["bzj", "helix_tmp_0000019fcb6957de_0000000000000012"],
                "sjx" => &["sjx", "helix_tmp_0000019fd1c566c6_0000000000000007"],
                _ => &[target],
            };
            aliases.iter().any(|alias| {
                msg_id.contains(alias)
                    || temporary_id.contains(alias)
                    || *alias == msg_id
                    || *alias == temporary_id
            })
        })
}

/// 判断 StorageOp 是否属于目标消息，非 message 操作保留事务上下文摘要。
fn storage_target_allowed(op: &StorageOp, ops: &[StorageOp]) -> bool {
    if !matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")
        && !matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "message")
    {
        return true;
    }
    storage_target_enabled(&storage_key_hint(op), &storage_temporary_id_hint(op, ops))
}

struct ReadbackRequest {
    operation_index: usize,
    channel_id: String,
    event_seq: u64,
    key_col: &'static str,
    key: SqlValue,
    temporary_id: String,
    columns: Vec<String>,
}

const FORENSIC_MESSAGE_COLUMNS: &[&str] = &[
    "expedite_map",
    "reply_messages",
    "reply_count",
    "reply_id",
    "reply_root_id",
    "reply_first_level_id",
    "replied_message",
];

/// 将字段消失问题涉及的富字段加入每次目标 message readback，保持值本身脱敏。
fn forensic_readback_columns(mut columns: Vec<String>) -> Vec<String> {
    columns.extend(
        FORENSIC_MESSAGE_COLUMNS
            .iter()
            .map(|column| (*column).to_string()),
    );
    columns
}

/// 提交后只回读本批 message 操作涉及的行，并按实际列动态输出摘要。
fn read_back_messages(
    conn: &rusqlite::Connection,
    context: &Option<StorageOperationContext>,
    ops: &[StorageOp],
) {
    let mut requests = Vec::new();
    for (operation_index, op) in ops.iter().enumerate() {
        if !storage_target_allowed(op, ops) {
            continue;
        }
        match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                if let Some(row) = spec.rows.first() {
                    if let Some((_, key)) = row.iter().find(|(column, _)| column == "temporary_id")
                    {
                        requests.push(ReadbackRequest {
                            operation_index,
                            channel_id: storage_channel_id_hint(op, ops),
                            event_seq: storage_event_seq_hint(op, ops),
                            key_col: "temporary_id",
                            key: key.clone(),
                            temporary_id: storage_temporary_id_hint(op, ops),
                            columns: forensic_readback_columns(
                                row.iter().map(|(column, _)| column.clone()).collect(),
                            ),
                        });
                    }
                }
            }
            StorageOp::BatchUpdate(spec) if spec.table == "message" => {
                for key in &spec.key_vals {
                    requests.push(ReadbackRequest {
                        operation_index,
                        channel_id: storage_channel_id_hint(op, ops),
                        event_seq: storage_event_seq_hint(op, ops),
                        key_col: spec.key_col,
                        key: key.clone(),
                        temporary_id: storage_temporary_id_hint(op, ops),
                        columns: forensic_readback_columns(
                            spec.patch
                                .iter()
                                .map(|(column, _)| column.clone())
                                .collect(),
                        ),
                    });
                }
            }
            _ => {}
        }
    }
    for request in requests {
        read_back_message(conn, context, request);
    }
}

/// 回读单条 message 行并输出 field presence/length/hash。
fn read_back_message(
    conn: &rusqlite::Connection,
    context: &Option<StorageOperationContext>,
    request: ReadbackRequest,
) {
    let columns = request
        .columns
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return;
    }
    let projection = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {projection} FROM message WHERE {} = ? LIMIT 1",
        quote_identifier(request.key_col)
    );
    let params = sql_values(std::iter::once(&request.key));
    let result = conn.query_row(&sql, params_from_iter(params.iter()), |row| {
        let mut presence = serde_json::Map::new();
        let mut lengths = serde_json::Map::new();
        let mut hashes = serde_json::Map::new();
        for (index, column) in columns.iter().enumerate() {
            let value = row.get_ref(index).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            let present = value_ref_present(value);
            presence.insert(column.clone(), serde_json::Value::Bool(present));
            if present {
                if let Some(length) = value_ref_length(value) {
                    lengths.insert(column.clone(), serde_json::Value::from(length));
                }
                hashes.insert(
                    column.clone(),
                    serde_json::Value::String(value_ref_hash(value)),
                );
            }
        }
        Ok(serde_json::json!({
            "field_presence": presence,
            "field_lengths": lengths,
            "field_hashes": hashes,
        }))
    });
    let corr = context.as_ref().and_then(|value| value.corr).unwrap_or(0);
    let track_id = context
        .as_ref()
        .and_then(|value| value.track_id.as_deref())
        .unwrap_or("helix-storage");
    match result {
        Ok(metadata) => tracing::info!(target: "offline_sync",
            hop = "sync.sqlite.readback",
            corr,
            upstream_corr = corr,
            track_id,
            channel_id = request.channel_id.as_str(),
            event_seq = request.event_seq,
            event_type = "readback",
            msg_id = storage_value_hint(&request.key),
            temporary_id = request.temporary_id.as_str(),
            source = "helix-driver-host",
            operation_id = format!("sync-readback:{corr}:{}", request.operation_index),
            row_found = true,
            field_presence = %metadata["field_presence"],
            field_lengths = %metadata["field_lengths"],
            field_hashes = %metadata["field_hashes"],
            "同步消息落库回读完成"
        ),
        Err(rusqlite::Error::QueryReturnedNoRows) => tracing::warn!(target: "offline_sync",
            hop = "sync.sqlite.readback",
            corr,
            upstream_corr = corr,
            track_id,
            channel_id = request.channel_id.as_str(),
            event_seq = request.event_seq,
            event_type = "readback",
            msg_id = storage_value_hint(&request.key),
            temporary_id = request.temporary_id.as_str(),
            source = "helix-driver-host",
            operation_id = format!("sync-readback:{corr}:{}", request.operation_index),
            row_found = false,
            "同步消息落库回读未找到目标行"
        ),
        Err(error) => tracing::error!(target: "offline_sync",
            hop = "sync.sqlite.readback_failed",
            corr,
            upstream_corr = corr,
            track_id,
            channel_id = request.channel_id.as_str(),
            event_seq = request.event_seq,
            event_type = "readback",
            msg_id = storage_value_hint(&request.key),
            temporary_id = request.temporary_id.as_str(),
            source = "helix-driver-host",
            operation_id = format!("sync-readback:{corr}:{}", request.operation_index),
            row_found = false,
            error = ?error,
            "同步消息落库回读失败"
        ),
    }
}

/// 为动态列名构造安全 SQL 标识符，StorageOp 列名均由 Helix 业务层提供。
fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn storage_value_hint(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(value) => value.clone(),
        SqlValue::Integer(value) => value.to_string(),
        _ => String::new(),
    }
}

fn value_ref_present(value: ValueRef<'_>) -> bool {
    match value {
        ValueRef::Null => false,
        ValueRef::Text(value) | ValueRef::Blob(value) => !value.is_empty(),
        ValueRef::Integer(value) => value != 0,
        ValueRef::Real(value) => value != 0.0,
    }
}

fn value_ref_length(value: ValueRef<'_>) -> Option<usize> {
    match value {
        ValueRef::Text(value) | ValueRef::Blob(value) => Some(value.len()),
        _ => None,
    }
}

fn value_ref_hash(value: ValueRef<'_>) -> String {
    let bytes = match value {
        ValueRef::Null => &[][..],
        ValueRef::Text(value) | ValueRef::Blob(value) => value,
        ValueRef::Integer(value) => return sha256(value.to_string().as_bytes()),
        ValueRef::Real(value) => return sha256(value.to_string().as_bytes()),
    };
    sha256(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{forensic_readback_columns, FORENSIC_MESSAGE_COLUMNS};

    /// 确保正文补丁的 readback 不会漏掉字段消失取证所需的富字段。
    #[test]
    fn forensic_readback_always_includes_reply_and_expedite_fields() {
        let columns = forensic_readback_columns(vec!["message".to_string()]);
        for column in FORENSIC_MESSAGE_COLUMNS {
            assert!(columns.iter().any(|candidate| candidate == column));
        }
    }
}
