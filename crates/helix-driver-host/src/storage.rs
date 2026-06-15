//! Shared rusqlite storage implementation for PC Tauri and Flutter FFI.

use std::sync::{Arc, Mutex};

use helix_core::effect::{
    GetSpec, MonotonicUpsertSpec, Row as HelixRow, ScanSpec, SqlValue, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{params_from_iter, Connection, OpenFlags};

#[derive(Clone)]
pub struct HostStorage {
    conn: Arc<Mutex<Connection>>,
}

impl HostStorage {
    pub async fn open_sqlite_url(db_url: &str) -> Result<Self, PortError> {
        let target = sqlite_target_from_url(db_url);
        let conn = tokio::task::spawn_blocking(move || {
            let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
            Connection::open_with_flags(target, flags).map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)??;

        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.migrate().await?;
        Ok(storage)
    }

    pub async fn execute_raw(&self, sql: &'static str) -> Result<(), PortError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(map_lock_err)?;
            conn.execute_batch(sql).map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn migrate(&self) -> Result<(), PortError> {
        self.execute_raw(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS _helix_monotonic
             (scope_key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL DEFAULT 0);",
        )
        .await
    }
}

#[async_trait::async_trait]
impl Storage for HostStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        if spec.rows.is_empty() {
            return Ok(());
        }

        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().map_err(map_lock_err)?;
            let col_names: Vec<&str> = spec.rows[0].iter().map(|(k, _)| k.as_str()).collect();
            let cols_sql = col_names.join(", ");
            let placeholders = col_names.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

            let sql = match spec.conflict_key {
                Some(key) => {
                    let update_cols: Vec<String> = col_names
                        .iter()
                        .filter(|&&c| c != key)
                        .map(|c| format!("{c} = excluded.{c}"))
                        .collect();
                    if update_cols.is_empty() {
                        format!(
                            "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({key}) DO NOTHING",
                            table = spec.table,
                            cols = cols_sql,
                            placeholders = placeholders,
                        )
                    } else {
                        format!(
                            "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({key}) DO UPDATE SET {updates}",
                            table = spec.table,
                            cols = cols_sql,
                            placeholders = placeholders,
                            updates = update_cols.join(", "),
                        )
                    }
                }
                None => format!(
                    "INSERT OR REPLACE INTO {table} ({cols}) VALUES ({placeholders})",
                    table = spec.table,
                    cols = cols_sql,
                    placeholders = placeholders,
                ),
            };

            let tx = conn.transaction().map_err(map_sqlite_err)?;
            {
                let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
                for row in &spec.rows {
                    let values = sql_values(row.iter().map(|(_, value)| value));
                    stmt.execute(params_from_iter(values.iter()))
                        .map_err(map_sqlite_err)?;
                }
            }
            tx.commit().map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: HelixRow,
    ) -> Result<(), PortError> {
        if key_vals.is_empty() || patch.is_empty() {
            return Ok(());
        }

        let conn = Arc::clone(&self.conn);
        let table = table.to_string();
        let key_col = key_col.to_string();
        let key_vals = key_vals.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(map_lock_err)?;
            let set_clause: Vec<String> = patch.iter().map(|(k, _)| format!("{k} = ?")).collect();
            let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();
            let sql = format!(
                "UPDATE {table} SET {set} WHERE {key_col} IN ({in_vals})",
                set = set_clause.join(", "),
                in_vals = in_placeholders.join(", "),
            );

            let mut values = sql_values(patch.iter().map(|(_, value)| value));
            values.extend(sql_values(key_vals.iter()));
            conn.execute(&sql, params_from_iter(values.iter()))
                .map(|_| ())
                .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(map_lock_err)?;
            conn.execute(
                "INSERT INTO _helix_monotonic (scope_key, value) VALUES (?, ?) \
                 ON CONFLICT(scope_key) DO UPDATE SET value = excluded.value \
                 WHERE excluded.value > _helix_monotonic.value",
                (&spec.scope_key, spec.value),
            )
            .map(|_| ())
            .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(map_lock_err)?;
            let sql = format!(
                "SELECT * FROM {table} WHERE {key_col} = ?",
                table = spec.table,
                key_col = spec.key_col,
            );
            let values = sql_values(std::iter::once(&spec.key_val));
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let mut rows = stmt
                .query(params_from_iter(values.iter()))
                .map_err(map_sqlite_err)?;
            match rows.next().map_err(map_sqlite_err)? {
                Some(row) => convert_row(row).map(Some),
                None => Ok(None),
            }
        })
        .await
        .map_err(map_join_err)?
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        const HARD_LIMIT: u32 = 10_000;
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(map_lock_err)?;
            let limit = spec.limit.unwrap_or(HARD_LIMIT).min(HARD_LIMIT);
            let sql = format!("SELECT * FROM {table} LIMIT {limit}", table = spec.table);
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let mut rows = stmt.query([]).map_err(map_sqlite_err)?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(map_sqlite_err)? {
                out.push(convert_row(row)?);
            }
            Ok(out)
        })
        .await
        .map_err(map_join_err)?
    }
}

fn sqlite_target_from_url(db_url: &str) -> String {
    let Some(rest) = db_url.strip_prefix("sqlite:") else {
        return db_url.to_string();
    };

    if rest.starts_with("file:") {
        return rest.to_string();
    }

    let without_query = rest.split('?').next().unwrap_or(rest);
    if without_query.starts_with("///") {
        format!("/{}", without_query.trim_start_matches('/'))
    } else {
        without_query.to_string()
    }
}

fn sql_values<'a>(values: impl IntoIterator<Item = &'a SqlValue>) -> Vec<Value> {
    values
        .into_iter()
        .map(|value| match value {
            SqlValue::Text(s) => Value::Text(s.clone()),
            SqlValue::Integer(i) => Value::Integer(*i),
            SqlValue::Real(f) => Value::Real(*f),
            SqlValue::Null => Value::Null,
        })
        .collect()
}

fn convert_row(row: &rusqlite::Row<'_>) -> Result<HelixRow, PortError> {
    let row_ref = row.as_ref();
    let mut out = Vec::with_capacity(row_ref.column_count());
    for idx in 0..row_ref.column_count() {
        let name = row_ref
            .column_name(idx)
            .map_err(map_sqlite_err)?
            .to_string();
        let value = match row.get_ref(idx).map_err(map_sqlite_err)? {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(i) => SqlValue::Integer(i),
            ValueRef::Real(f) => SqlValue::Real(f),
            ValueRef::Text(s) => SqlValue::Text(String::from_utf8_lossy(s).into_owned()),
            ValueRef::Blob(b) => SqlValue::Text(String::from_utf8_lossy(b).into_owned()),
        };
        out.push((name, value));
    }
    Ok(out)
}

fn map_sqlite_err(e: impl std::fmt::Display) -> PortError {
    PortError::Storage(e.to_string())
}

fn map_lock_err<T>(_: std::sync::PoisonError<T>) -> PortError {
    PortError::Storage("sqlite connection lock poisoned".to_string())
}

fn map_join_err(e: tokio::task::JoinError) -> PortError {
    PortError::Storage(format!("sqlite blocking worker failed: {e}"))
}
