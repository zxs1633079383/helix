use helix_core::effect::{Row as HelixRow, SqlValue};
use helix_core::PortError;
use rusqlite::types::{Value, ValueRef};

use super::connection::map_sqlite_err;

pub(super) fn sql_values<'a>(values: impl IntoIterator<Item = &'a SqlValue>) -> Vec<Value> {
    values
        .into_iter()
        .map(|value| match value {
            SqlValue::Text(s) => Value::Text(s.clone()),
            SqlValue::Integer(i) => Value::Integer(*i),
            SqlValue::Real(f) => Value::Real(*f),
            SqlValue::Blob(b) => Value::Blob(b.clone()),
            SqlValue::Null => Value::Null,
        })
        .collect()
}

pub(super) fn convert_row(row: &rusqlite::Row<'_>) -> Result<HelixRow, PortError> {
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
            ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
        };
        out.push((name, value));
    }
    Ok(out)
}
