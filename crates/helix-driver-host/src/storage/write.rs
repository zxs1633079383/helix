use std::borrow::Cow;
use std::sync::Arc;

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GuardedBumpSpec, MonotonicUpsertSpec, SqlValue, StorageOp,
    UpsertSpec,
};
use helix_core::PortError;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use rusqlite::Transaction;

use super::connection::{map_join_err, map_lock_err, map_sqlite_err};
use super::trace::{current_storage_trace_context, trace_sql, StorageTraceContext};
use super::values::sql_values;
use super::HostStorage;

pub(super) async fn batch_upsert(storage: &HostStorage, spec: UpsertSpec) -> Result<(), PortError> {
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
                let values = sql_values(row.iter().map(|(_, value)| value));
                let _span = trace_sql(&trace, "UPSERT", Some(spec.table), &sql, &values);
                stmt.execute(params_from_iter(values.iter()))
                    .map_err(map_sqlite_err)?;
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
    let trace = current_storage_trace_context();
    tokio::task::spawn_blocking(move || {
        let mut conn = writer.lock().map_err(map_lock_err)?;
        let tx = conn.transaction().map_err(map_sqlite_err)?;
        for op in &ops {
            atomic_write_op(&tx, op, &trace)?;
        }
        tx.commit().map_err(map_sqlite_err)
    })
    .await
    .map_err(map_join_err)?
}

fn atomic_write_op(
    tx: &Transaction<'_>,
    op: &StorageOp,
    trace: &Option<StorageTraceContext>,
) -> Result<(), PortError> {
    match op {
        StorageOp::BatchUpsert(spec) => atomic_batch_upsert(tx, spec, trace),
        StorageOp::MonotonicUpsert(spec) => atomic_monotonic_upsert(tx, spec, trace),
        StorageOp::BatchUpdate(spec) => atomic_batch_update(tx, spec, trace),
        StorageOp::GuardedBump(spec) => atomic_guarded_bump(tx, spec, trace),
        StorageOp::BatchDelete(spec) => atomic_batch_delete(tx, spec, trace),
        StorageOp::Get(_) | StorageOp::Scan(_) => Err(PortError::Storage(
            "atomic storage write does not permit read operations".to_string(),
        )),
    }
}

fn atomic_batch_upsert(
    tx: &Transaction<'_>,
    spec: &UpsertSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<(), PortError> {
    if spec.rows.is_empty() {
        return Ok(());
    }
    let sql = crate::upsert_sql::upsert_sql_for_shape(spec);
    let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
    for row in &spec.rows {
        let values = sql_values(row.iter().map(|(_, value)| value));
        let _span = trace_sql(trace, "UPSERT", Some(spec.table), &sql, &values);
        stmt.execute(params_from_iter(values.iter()))
            .map_err(map_sqlite_err)?;
    }
    Ok(())
}

fn atomic_monotonic_upsert(
    tx: &Transaction<'_>,
    spec: &MonotonicUpsertSpec,
    trace: &Option<StorageTraceContext>,
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
        .map(|_| ())
        .map_err(map_sqlite_err)
}

fn atomic_batch_update(
    tx: &Transaction<'_>,
    spec: &BatchUpdateSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<(), PortError> {
    if spec.key_vals.is_empty() || spec.patch.is_empty() {
        return Ok(());
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
        .map(|_| ())
        .map_err(map_sqlite_err)
}

fn atomic_guarded_bump(
    tx: &Transaction<'_>,
    spec: &GuardedBumpSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<(), PortError> {
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
        .map(|_| ())
        .map_err(map_sqlite_err)
}

fn atomic_batch_delete(
    tx: &Transaction<'_>,
    spec: &BatchDeleteSpec,
    trace: &Option<StorageTraceContext>,
) -> Result<(), PortError> {
    if spec.key_vals.is_empty() {
        return Ok(());
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
        .map(|_| ())
        .map_err(map_sqlite_err)
}
