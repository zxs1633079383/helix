use std::cmp::Ordering;

use helix_core::effect::{
    GetSpec, Row as HelixRow, ScanSpec, ScopedGetSpec, ScopedMaxSpec, ScopedScanSpec,
    SortDirection, SqlValue,
};
use helix_core::PortError;
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use super::connection::map_sqlite_err;
use super::trace::{current_storage_trace_context, trace_sql};
use super::values::{convert_row, sql_values};
use super::HostStorage;

const MAX_BOUND_VALUES_PER_QUERY: usize = 900;
const MAX_SCOPED_SCAN_LIMIT: usize = 10_000;

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

/// 以 scope + key 两个绑定参数精确读取复合主键行。
pub(super) async fn scoped_get(
    storage: &HostStorage,
    spec: ScopedGetSpec,
) -> Result<Option<HelixRow>, PortError> {
    let trace = current_storage_trace_context();
    storage
        .with_reader(move |conn| {
            let sql = format!(
                "SELECT * FROM {table} WHERE {scope_col} = ? AND {key_col} = ?",
                table = spec.table,
                scope_col = spec.scope_col,
                key_col = spec.key_col,
            );
            let values = sql_values([&spec.scope_val, &spec.key_val].into_iter());
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

/// Read the maximum value inside a bounded scope set.
///
/// SQLite's variable limit is driver-specific, so the scope set is split into conservative
/// chunks. Each chunk applies `WHERE scope_col IN (...)` before `MAX(value_col)`; the chunk
/// maxima are merged without ever falling back to an unscoped read.
pub(super) async fn scoped_max(
    storage: &HostStorage,
    spec: ScopedMaxSpec,
) -> Result<Option<HelixRow>, PortError> {
    if spec.scope_values.is_empty() {
        return Ok(None);
    }

    let trace = current_storage_trace_context();
    storage
        .with_reader(move |conn| {
            let transaction = conn.unchecked_transaction().map_err(map_sqlite_err)?;
            let mut maximum = None;
            for scope_chunk in spec.scope_values.chunks(MAX_BOUND_VALUES_PER_QUERY) {
                let placeholders = std::iter::repeat("?")
                    .take(scope_chunk.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT MAX({value_col}) AS {result_alias} FROM {table} WHERE {scope_col} IN ({placeholders})",
                    value_col = spec.value_col,
                    result_alias = spec.result_alias,
                    table = spec.table,
                    scope_col = spec.scope_col,
                );
                let bind = sql_values(scope_chunk.iter());
                let mut stmt = transaction.prepare_cached(&sql).map_err(map_sqlite_err)?;
                let _span = trace_sql(&trace, "SELECT", Some(spec.table), &sql, &bind);
                let mut rows = stmt
                    .query(params_from_iter(bind.iter()))
                    .map_err(map_sqlite_err)?;
                let Some(row) = rows.next().map_err(map_sqlite_err)? else {
                    continue;
                };
                let chunk_row = convert_row(row)?;
                let chunk_max = chunk_row
                    .iter()
                    .find(|(column, _)| column == spec.result_alias)
                    .map(|(_, value)| value.clone());
                maximum = merge_maximum(maximum, chunk_max);
            }

            transaction.commit().map_err(map_sqlite_err)?;
            Ok(maximum.map(|value| vec![(spec.result_alias.to_string(), value)]))
        })
        .await
}

/// Read all rows inside a bounded scope set without silently truncating a roster.
///
/// Each chunk applies the `IN` predicate before a bounded `LIMIT remaining + 1`. The extra
/// sentinel row lets the adapter distinguish an exact result from an over-limit result and
/// fail closed. Empty scope values never touch the table.
pub(super) async fn scoped_scan(
    storage: &HostStorage,
    spec: ScopedScanSpec,
) -> Result<Vec<HelixRow>, PortError> {
    if spec.scope_values.is_empty() {
        return Ok(Vec::new());
    }
    if spec.limit > MAX_SCOPED_SCAN_LIMIT {
        return Err(PortError::Storage(format!(
            "scoped scan limit exceeds driver maximum {MAX_SCOPED_SCAN_LIMIT}"
        )));
    }

    let ScopedScanSpec {
        table,
        scope_col,
        scope_values,
        limit,
    } = spec;
    let mut scope_values = scope_values;
    scope_values.sort_by(compare_sql_values);
    scope_values.dedup_by(|left, right| compare_sql_values(left, right) == Ordering::Equal);

    let trace = current_storage_trace_context();
    storage
        .with_reader(move |conn| {
            // Keep every chunk on one deferred read transaction so the bounded roster is
            // evaluated against one SQLite snapshot even when a writer commits between chunks.
            let transaction = conn.unchecked_transaction().map_err(map_sqlite_err)?;
            let mut remaining = limit;
            let mut output = Vec::new();
            for scope_chunk in scope_values.chunks(MAX_BOUND_VALUES_PER_QUERY) {
                let placeholders = std::iter::repeat("?")
                    .take(scope_chunk.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let fetch_limit = remaining + 1;
                let sql = format!(
                    "SELECT * FROM {table} WHERE {scope_col} IN ({placeholders}) LIMIT {fetch_limit}",
                    table = table,
                    scope_col = scope_col,
                );
                let bind = sql_values(scope_chunk.iter());
                let mut stmt = transaction.prepare_cached(&sql).map_err(map_sqlite_err)?;
                let _span = trace_sql(&trace, "SELECT", Some(table), &sql, &bind);
                let mut cursor = stmt
                    .query(params_from_iter(bind.iter()))
                    .map_err(map_sqlite_err)?;
                let mut chunk_rows = Vec::new();
                while let Some(row) = cursor.next().map_err(map_sqlite_err)? {
                    chunk_rows.push(convert_row(row)?);
                }
                if chunk_rows.len() > remaining {
                    return Err(PortError::Storage(
                        "scoped scan result exceeds explicit limit".to_string(),
                    ));
                }
                remaining -= chunk_rows.len();
                output.extend(chunk_rows);
            }
            transaction.commit().map_err(map_sqlite_err)?;
            Ok(output)
        })
        .await
}

fn merge_maximum(current: Option<SqlValue>, candidate: Option<SqlValue>) -> Option<SqlValue> {
    let Some(candidate) = candidate.filter(|value| !matches!(value, SqlValue::Null)) else {
        return current;
    };
    let Some(current) = current else {
        return Some(candidate);
    };
    if compare_sql_values(&candidate, &current) == Ordering::Greater {
        Some(candidate)
    } else {
        Some(current)
    }
}

fn compare_sql_values(left: &SqlValue, right: &SqlValue) -> Ordering {
    match (left, right) {
        (SqlValue::Integer(left), SqlValue::Integer(right)) => left.cmp(right),
        (SqlValue::Real(left), SqlValue::Real(right)) => {
            left.partial_cmp(right).unwrap_or(Ordering::Equal)
        }
        (SqlValue::Integer(left), SqlValue::Real(right)) => {
            (*left as f64).partial_cmp(right).unwrap_or(Ordering::Equal)
        }
        (SqlValue::Real(left), SqlValue::Integer(right)) => left
            .partial_cmp(&(*right as f64))
            .unwrap_or(Ordering::Equal),
        (SqlValue::Text(left), SqlValue::Text(right)) => left.cmp(right),
        (SqlValue::Blob(left), SqlValue::Blob(right)) => left.cmp(right),
        (SqlValue::Null, SqlValue::Null) => Ordering::Equal,
        (SqlValue::Null, _) => Ordering::Less,
        (_, SqlValue::Null) => Ordering::Greater,
        (left, right) => sql_type_rank(left).cmp(&sql_type_rank(right)),
    }
}

fn sql_type_rank(value: &SqlValue) -> u8 {
    match value {
        SqlValue::Null => 0,
        SqlValue::Integer(_) | SqlValue::Real(_) => 1,
        SqlValue::Text(_) => 2,
        SqlValue::Blob(_) => 3,
    }
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
            let order_clause = if spec.order_by.is_empty() {
                String::new()
            } else {
                let terms = spec
                    .order_by
                    .iter()
                    .map(|order| {
                        let direction = match order.direction {
                            SortDirection::Asc => "ASC",
                            SortDirection::Desc => "DESC",
                        };
                        format!("{} {direction}", order.column)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(" ORDER BY {terms}")
            };
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
