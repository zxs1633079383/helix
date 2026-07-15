use std::sync::{Arc, Mutex};

use helix_core::PortError;
use rusqlite::{Connection, OpenFlags};
use tokio::sync::Semaphore;

pub(super) const READ_POOL_SIZE: usize = 4;

pub(super) struct ReadPool {
    pub(super) permits: Arc<Semaphore>,
    pub(super) idle: Mutex<Vec<Connection>>,
}

impl ReadPool {
    pub(super) fn new(conns: Vec<Connection>) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(READ_POOL_SIZE)),
            idle: Mutex::new(conns),
        }
    }
}

/// 打开写连接：READ_WRITE | CREATE，NO_MUTEX（连接被 Rust Mutex 独占）+ URI。
pub(super) fn open_writer(target: &str) -> Result<Connection, PortError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(target, flags).map_err(map_sqlite_err)
}

/// 打开只读连接：READ_ONLY + NO_MUTEX + URI，并保留 5s busy timeout。
pub(super) fn open_reader(target: &str) -> Result<Connection, PortError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(target, flags).map_err(map_sqlite_err)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(map_sqlite_err)?;
    Ok(conn)
}

pub(super) fn target_supports_shared_readers(target: &str) -> bool {
    let is_memory = target.contains(":memory:") || target.contains("mode=memory");
    if !is_memory {
        return true;
    }
    target.contains("cache=shared")
}

pub(super) fn sqlite_target_from_url(db_url: &str) -> String {
    let Some(rest) = db_url.strip_prefix("sqlite:") else {
        return db_url.to_string();
    };

    if rest.starts_with("file:") {
        return rest.to_string();
    }

    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };

    let normalized_path = if path.starts_with("///") {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path.to_string()
    };

    match query {
        Some(q) => format!("{normalized_path}?{q}"),
        None => normalized_path,
    }
}

pub(super) fn map_sqlite_err(e: impl std::fmt::Display) -> PortError {
    PortError::Storage(e.to_string())
}

pub(super) fn map_lock_err<T>(_: std::sync::PoisonError<T>) -> PortError {
    PortError::Storage("sqlite connection lock poisoned".to_string())
}

pub(super) fn map_join_err(e: tokio::task::JoinError) -> PortError {
    PortError::Storage(format!("sqlite blocking worker failed: {e}"))
}
