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
    let reader_target = read_only_target(target);
    let conn = Connection::open_with_flags(reader_target, flags).map_err(map_sqlite_err)?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(map_sqlite_err)?;
    Ok(conn)
}

/// 只读连接移除访问模式参数，同时保留共享内存库和其他 URI 参数。
fn read_only_target(target: &str) -> String {
    let Some((path, query)) = target.split_once('?') else {
        return target.to_string();
    };

    let read_query = query_without_access_mode(query);

    if read_query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", read_query.join("&"))
    }
}

pub(super) fn target_supports_shared_readers(target: &str) -> bool {
    let is_memory = target.contains(":memory:") || target.contains("mode=memory");
    if !is_memory {
        return true;
    }
    target.contains("cache=shared")
}

/// 统一 SQLite URL 的 writer/reader 目标，避免 `?query` 被当作普通路径字符。
pub(super) fn sqlite_target_from_url(db_url: &str) -> String {
    let target = db_url.strip_prefix("sqlite:").unwrap_or(db_url);

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };

    let normalized_path = normalize_sqlite_path(path);

    let Some(query) = query else {
        return normalized_path;
    };

    let writer_query = query_without_write_create_mode(query);
    if writer_query.is_empty() {
        return normalized_path;
    }

    let uri_path = if normalized_path.starts_with("file:") {
        normalized_path
    } else if normalized_path == ":memory:" {
        "file::memory:".to_string()
    } else {
        format!("file:{normalized_path}")
    };
    format!("{uri_path}?{}", writer_query.join("&"))
}

/// 从 writer/reader 共用目标移除访问模式，唯独保留 SQLite 共享内存所需的 `mode=memory`。
fn query_without_access_mode(query: &str) -> Vec<&str> {
    query
        .split('&')
        .filter(|part| {
            part.split_once('=')
                .map_or(true, |(key, value)| key != "mode" || value == "memory")
        })
        .collect()
}

/// 为 writer 只移除冗余 `mode=rwc`，不改变 `mode=rw/ro` 的原有访问语义。
fn query_without_write_create_mode(query: &str) -> Vec<&str> {
    query
        .split('&')
        .filter(|part| {
            part.split_once('=')
                .map_or(true, |(key, value)| !(key == "mode" && value == "rwc"))
        })
        .collect()
}

/// 将 Unix、drive 和 UNC 路径收敛为 SQLite `file:` URI 可接受的形态。
fn normalize_sqlite_path(path: &str) -> String {
    let slash_path =
        if path.starts_with("file:") || is_windows_drive_path(path) || path.starts_with("\\\\") {
            path.replace('\\', "/")
        } else {
            path.to_string()
        };

    if let Some(file_path) = slash_path.strip_prefix("file:") {
        if is_windows_drive_path(file_path.trim_start_matches('/')) {
            return format!("file:{}", file_path.trim_start_matches('/'));
        }
        return slash_path;
    }

    if is_windows_drive_path(slash_path.trim_start_matches('/')) {
        return format!("file:{}", slash_path.trim_start_matches('/'));
    }

    if slash_path.starts_with("///") {
        let without_slashes = slash_path.trim_start_matches('/');
        if is_windows_drive_path(without_slashes) {
            return format!("file:{without_slashes}");
        }
        return format!("/{without_slashes}");
    }

    if slash_path.starts_with("//") {
        let without_slashes = slash_path.trim_start_matches('/');
        if is_windows_drive_path(without_slashes) {
            return format!("file:{without_slashes}");
        }
        return format!("file://{without_slashes}");
    }

    slash_path
}

/// 识别 `C:/...` 和 `C:\\...`，避免把 Windows drive 路径误当成 Unix 路径。
fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{open_reader, open_writer, read_only_target, sqlite_target_from_url};

    /// 为 writer/reader 回归测试生成隔离的临时 SQLite 路径。
    fn temp_db_path(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "helix-storage-{label}-{}-{sequence}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        path
    }

    /// 删除回归测试产生的主库和 WAL/SHM sidecar。
    fn remove_temp_db(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn read_only_target_drops_write_mode_but_keeps_other_uri_options() {
        assert_eq!(
            read_only_target("file:/tmp/tenant.db?mode=rwc"),
            "file:/tmp/tenant.db"
        );
        assert_eq!(
            read_only_target("file:/tmp/tenant.db?mode=rwc&cache=shared"),
            "file:/tmp/tenant.db?cache=shared"
        );
        assert_eq!(
            read_only_target("file:/tmp/tenant.db?cache=shared&mode=rwc"),
            "file:/tmp/tenant.db?cache=shared"
        );
        assert_eq!(
            read_only_target("file::memory:?mode=memory&cache=shared"),
            "file::memory:?mode=memory&cache=shared"
        );
        assert_eq!(
            read_only_target("file:/tmp/tenant.db?mode=ro"),
            "file:/tmp/tenant.db"
        );
        assert_eq!(
            read_only_target("file:/tmp/tenant.db?mode=rw"),
            "file:/tmp/tenant.db"
        );
        assert_eq!(
            read_only_target("file:/tmp/tenant.db"),
            "file:/tmp/tenant.db"
        );
    }

    #[test]
    fn sqlite_target_from_url_normalizes_uri_and_windows_paths() {
        assert_eq!(
            sqlite_target_from_url("sqlite:/tmp/tenant.db?mode=rwc"),
            "/tmp/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:///tmp/tenant.db?mode=rwc&cache=shared"),
            "file:/tmp/tenant.db?cache=shared"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:file:/tmp/tenant.db?mode=rwc"),
            "file:/tmp/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:file:/tmp/tenant.db?mode=ro"),
            "file:/tmp/tenant.db?mode=ro"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:file:/tmp/tenant.db?mode=rw"),
            "file:/tmp/tenant.db?mode=rw"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:///C:/Users/venus/tenant.db?mode=rwc"),
            "file:C:/Users/venus/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:file:C:\\Users\\venus\\tenant.db?mode=rwc"),
            "file:C:/Users/venus/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:C:\\Users\\venus\\tenant.db?mode=rwc"),
            "file:C:/Users/venus/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:\\\\server\\share\\tenant.db?mode=rwc"),
            "file://server/share/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite://server/share/tenant.db?mode=rwc"),
            "file://server/share/tenant.db"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite::memory:?cache=shared"),
            "file::memory:?cache=shared"
        );
        assert_eq!(
            sqlite_target_from_url("sqlite:file:server/share/tenant.db?mode=rwc"),
            "file:server/share/tenant.db"
        );
    }

    #[test]
    fn writer_and_reader_share_sqlite_scheme_path_for_mode_rwc() {
        let path = temp_db_path("uri");
        let raw_url = format!("sqlite:{}?mode=rwc", path.display());
        let target = sqlite_target_from_url(&raw_url);
        let writer = open_writer(&target).expect("writer should open canonical target");
        writer
            .execute_batch(
                "CREATE TABLE sample (value INTEGER NOT NULL); INSERT INTO sample VALUES (42);",
            )
            .expect("writer should create and populate the table");

        let reader = open_reader(&target).expect("reader should open the same canonical target");
        let value: i64 = reader
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .expect("reader should see writer data");
        assert_eq!(value, 42);
        drop(reader);
        drop(writer);
        remove_temp_db(&path);
    }
}
