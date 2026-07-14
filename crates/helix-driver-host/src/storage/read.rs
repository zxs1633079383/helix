use helix_core::effect::{GetSpec, Row as HelixRow, ScanSpec};
use helix_core::PortError;
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use super::connection::map_sqlite_err;
use super::trace::{current_storage_trace_context, trace_sql};
use super::values::{convert_row, sql_values};
use super::HostStorage;

pub(super) async fn get(
    storage: &HostStorage,
    spec: GetSpec,
) -> Result<Option<HelixRow>, PortError> {
    let trace = current_storage_trace_context();
    storage
        .with_reader(move |conn| {
            let sql = format!(
                "SELECT * FROM {table} WHERE {key_col} = ?",
                table = spec.table,
                key_col = spec.key_col,
            );
            let values = sql_values(std::iter::once(&spec.key_val));
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let _span = trace_sql(&trace, "SELECT", Some(spec.table), &sql, &values);
            let mut rows = stmt
                .query(params_from_iter(values.iter()))
                .map_err(map_sqlite_err)?;
            match rows.next().map_err(map_sqlite_err)? {
                Some(row) => convert_row(row).map(Some),
                None => Ok(None),
            }
        })
        .await
}

pub(super) async fn scan(
    storage: &HostStorage,
    spec: ScanSpec,
) -> Result<Vec<HelixRow>, PortError> {
    const HARD_LIMIT: u32 = 10_000;
    let trace = current_storage_trace_context();
    storage
        .with_reader(move |conn| {
            let limit = spec.limit.unwrap_or(HARD_LIMIT).min(HARD_LIMIT);
            let where_clause = spec
                .filter
                .as_ref()
                .map_or(String::new(), |(c, _)| format!(" WHERE {c} = ?"));
            let order_clause = spec
                .order_desc_col
                .map_or(String::new(), |c| format!(" ORDER BY {c} DESC"));
            let sql = format!(
                "SELECT * FROM {table}{where_clause}{order_clause} LIMIT {limit}",
                table = spec.table,
            );
            let bind: Vec<Value> = spec
                .filter
                .as_ref()
                .map_or_else(Vec::new, |(_, val)| sql_values(std::iter::once(val)));
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let _span = trace_sql(&trace, "SELECT", Some(spec.table), &sql, &bind);
            let mut rows = stmt
                .query(params_from_iter(bind.iter()))
                .map_err(map_sqlite_err)?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(map_sqlite_err)? {
                out.push(convert_row(row)?);
            }
            Ok(out)
        })
        .await
}
