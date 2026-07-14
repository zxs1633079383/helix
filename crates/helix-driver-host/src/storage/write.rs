use std::borrow::Cow;
use std::sync::Arc;

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GuardedBumpSpec, MonotonicUpsertSpec, SqlValue, UpsertSpec,
};
use helix_core::PortError;
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use super::connection::{map_join_err, map_lock_err, map_sqlite_err};
use super::trace::{current_storage_trace_context, trace_sql};
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
