use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const TABLES: &[&str] = &["message", "channel", "channel_member", "channel_event_cursor"];
const FILTERS: &[&str] = &["channel_id", "id", "temporary_id", "event_seq"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectRequest {
    pub db: PathBuf,
    pub account_id: String,
    pub table: String,
    pub filter: Option<(String, String)>,
    pub limit: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("invalid inspect request: {0}")]
    Invalid(&'static str),
    #[error("sqlite inspection failed")]
    Sql(#[from] rusqlite::Error),
    #[error("cannot encode redacted report")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
pub struct InspectResult {
    pub account_id: String,
    pub db_path_sha256: String,
    pub table: String,
    pub row_count: usize,
    pub rows: Vec<serde_json::Value>,
    pub json: String,
}

impl InspectRequest {
    pub fn parse<I, S>(args: I) -> Result<Self, InspectError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let mut db = None;
        let mut account_id = None;
        let mut table = None;
        let mut limit = 50usize;
        let mut filter = None;
        let mut index = 0;
        while index < args.len() {
            let key = args[index].as_str();
            let value = args.get(index + 1).ok_or(InspectError::Invalid("missing argument value"))?.clone();
            match key {
                "--db" => db = Some(PathBuf::from(value)),
                "--account-id" => account_id = Some(value),
                "--table" => table = Some(value),
                "--limit" => limit = value.parse().map_err(|_| InspectError::Invalid("invalid limit"))?,
                "--channel-id" | "--id" | "--temporary-id" | "--event-seq" => {
                    if filter.is_some() { return Err(InspectError::Invalid("only one filter is allowed")); }
                    filter = Some((key.trim_start_matches("--").replace('-', "_"), value));
                }
                _ => return Err(InspectError::Invalid("unknown argument")),
            }
            index += 2;
        }
        let request = Self {
            db: db.ok_or(InspectError::Invalid("db is required"))?,
            account_id: account_id.ok_or(InspectError::Invalid("account id is required"))?,
            table: table.ok_or(InspectError::Invalid("table is required"))?,
            filter,
            limit,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), InspectError> {
        if !TABLES.contains(&self.table.as_str()) { return Err(InspectError::Invalid("table is not allowlisted")); }
        if self.limit == 0 || self.limit > 200 { return Err(InspectError::Invalid("limit must be between 1 and 200")); }
        if self.account_id.is_empty() || !self.db.to_string_lossy().contains(&self.account_id) { return Err(InspectError::Invalid("db must be scoped to requested account")); }
        if let Some((field, _)) = &self.filter {
            if !FILTERS.contains(&field.as_str()) { return Err(InspectError::Invalid("filter is not allowlisted")); }
        }
        Ok(())
    }
}

pub fn inspect(request: &InspectRequest) -> Result<InspectResult, InspectError> {
    request.validate()?;
    let connection = Connection::open_with_flags(&request.db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut sql = format!("SELECT * FROM {}", request.table);
    if let Some((field, _)) = &request.filter { sql.push_str(&format!(" WHERE {field} = ?1")); }
    sql.push_str(" LIMIT ?2");
    let mut statement = connection.prepare(&sql)?;
    let mut rows = match &request.filter {
        Some((_, value)) => statement.query(rusqlite::params![value, request.limit as i64])?,
        None => statement.query(rusqlite::params![request.limit as i64])?,
    };
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        let mut value = serde_json::Map::new();
        for index in 0..row.as_ref().column_count() {
            let name = row.as_ref().column_name(index)?;
            if matches!(name, "body" | "message" | "payload" | "content" | "text") { continue; }
            let item = row.get_ref(index)?;
            let rendered = match item {
                rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                rusqlite::types::ValueRef::Integer(value) => value.into(),
                rusqlite::types::ValueRef::Real(value) => value.into(),
                rusqlite::types::ValueRef::Text(value) => String::from_utf8_lossy(value).to_string().into(),
                rusqlite::types::ValueRef::Blob(_) => "[REDACTED]".into(),
            };
            value.insert(name.to_owned(), rendered);
        }
        values.push(serde_json::Value::Object(value));
    }
    let report = serde_json::json!({
        "accountId": request.account_id,
        "dbPathSha256": path_hash(&request.db),
        "table": request.table,
        "rowCount": values.len(),
        "rows": values,
    });
    Ok(InspectResult { account_id: request.account_id.clone(), db_path_sha256: path_hash(&request.db), table: request.table.clone(), row_count: values.len(), rows: values, json: serde_json::to_string(&report)? })
}

fn path_hash(path: &Path) -> String { format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes())) }
