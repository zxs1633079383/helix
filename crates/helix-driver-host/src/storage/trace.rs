use std::future::Future;
use std::path::PathBuf;

use rusqlite::types::Value;
use serde_json::json;

use crate::base64_encode;
use crate::otel::{HostOtelRuntime, HostSpanScope, TraceDirection};
use crate::trace::TraceCarrier;
use helix_core::Correlation;

const SQL_TRACE_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct StorageTraceContext {
    runtime: HostOtelRuntime,
    carrier: Option<TraceCarrier>,
}

/// Persist worker 的结构化诊断上下文；即使 OTel 未启用，也保留 corr/track 关联。
#[derive(Clone, Debug)]
pub struct StorageOperationContext {
    pub corr: Option<u64>,
    pub track_id: Option<String>,
}

impl StorageOperationContext {
    /// 从 Persist job 的 correlation 生成脱敏运行关联。
    pub fn new(corr: Option<Correlation>) -> Self {
        let corr = corr.map(|value| value.raw());
        let track_id = corr.map(|value| format!("helix-sync:{value}"));
        Self { corr, track_id }
    }
}

impl StorageTraceContext {
    pub fn new(runtime: HostOtelRuntime, carrier: Option<TraceCarrier>) -> Self {
        Self { runtime, carrier }
    }
}

tokio::task_local! {
    static STORAGE_TRACE_CONTEXT: StorageTraceContext;
    static STORAGE_OPERATION_CONTEXT: StorageOperationContext;
}

pub async fn with_storage_trace_context<F>(context: StorageTraceContext, future: F) -> F::Output
where
    F: Future,
{
    STORAGE_TRACE_CONTEXT.scope(context, future).await
}

/// 在 Persist worker 生命周期内绑定 corr，供 SQLite 写日志使用。
pub async fn with_storage_operation_context<F>(
    context: StorageOperationContext,
    future: F,
) -> F::Output
where
    F: Future,
{
    STORAGE_OPERATION_CONTEXT.scope(context, future).await
}

pub(super) fn current_storage_trace_context() -> Option<StorageTraceContext> {
    STORAGE_TRACE_CONTEXT.try_with(Clone::clone).ok()
}

pub(super) fn current_storage_operation_context() -> Option<StorageOperationContext> {
    STORAGE_OPERATION_CONTEXT.try_with(Clone::clone).ok()
}

/// 读取布尔环境开关，driver 侧允许 ambient 配置，业务 core 不读取环境。
pub(super) fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "on"))
        .unwrap_or(false)
}

/// 判断是否开启离线 sync 结构化存储日志。
pub(super) fn offline_sync_trace_enabled() -> bool {
    env_enabled("HELIX_OFFLINE_SYNC_TRACE")
}

/// 判断是否开启 WAL 元数据采集。
pub(super) fn offline_sync_wal_enabled() -> bool {
    env_enabled("HELIX_OFFLINE_SYNC_WAL")
}

/// 只读扫描 SQLite WAL header/frame，返回可与应用日志关联的元数据。
pub(super) fn wal_snapshot(db_target: &str) -> serde_json::Value {
    let Some(db_path) = sqlite_path(db_target) else {
        return serde_json::json!({"status": "unsupported_target"});
    };
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    let db_meta = std::fs::metadata(&db_path).ok();
    let wal_meta = std::fs::metadata(&wal_path).ok();
    let shm_meta = std::fs::metadata(&shm_path).ok();
    let mut result = serde_json::json!({
        "status": if wal_meta.is_some() { "ok" } else { "missing" },
        "db_path": db_path.display().to_string(),
        "wal_path": wal_path.display().to_string(),
        "shm_path": shm_path.display().to_string(),
        "db_bytes": db_meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
        "wal_bytes": wal_meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
        "shm_bytes": shm_meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
        "db_mtime_ms": metadata_mtime_ms(db_meta.as_ref()),
        "wal_mtime_ms": metadata_mtime_ms(wal_meta.as_ref()),
        "shm_mtime_ms": metadata_mtime_ms(shm_meta.as_ref()),
    });
    let Ok(bytes) = std::fs::read(&wal_path) else {
        return result;
    };
    if bytes.len() < 32 {
        result["status"] = serde_json::json!("invalid_or_truncated");
        return result;
    }
    let page_size = u32::from_be_bytes(bytes[8..12].try_into().unwrap_or_default());
    let page_size = if page_size == 1 {
        65_536
    } else {
        page_size.max(1)
    } as usize;
    let frame_size = page_size.saturating_add(24);
    let frame_count = if frame_size == 0 {
        0
    } else {
        bytes.len().saturating_sub(32) / frame_size
    };
    let mut commit_frames = Vec::new();
    let mut pages = Vec::new();
    for index in 0..frame_count {
        let offset = 32 + index * frame_size;
        let page = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default());
        let commit =
            u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap_or_default());
        pages.push(page);
        if commit != 0 {
            commit_frames.push(index + 1);
        }
    }
    const PAGE_TAIL_LIMIT: usize = 4096;
    let pages_truncated = pages.len() > PAGE_TAIL_LIMIT;
    if pages_truncated {
        pages = pages.split_off(pages.len() - PAGE_TAIL_LIMIT);
    }
    result["page_size"] = serde_json::json!(page_size);
    result["frame_count"] = serde_json::json!(frame_count);
    result["last_commit_frame"] = serde_json::json!(commit_frames.last().copied().unwrap_or(0));
    result["commit_frame_count"] = serde_json::json!(commit_frames.len());
    result["page_numbers_tail"] = serde_json::json!(pages);
    result["page_numbers_truncated"] = serde_json::json!(pages_truncated);
    result
}

/// 从 SQLite target 中提取 WAL/SHM 对应的本地数据库路径，不打开或修改连接。
fn sqlite_path(target: &str) -> Option<PathBuf> {
    let target = target.strip_prefix("file:").unwrap_or(target);
    let path = target.split('?').next()?.trim();
    if path.is_empty() || path == ":memory:" || path.starts_with("file::memory:") {
        return None;
    }
    Some(PathBuf::from(path))
}

/// 将文件修改时间转换为可与结构化日志关联的 Unix 毫秒时间戳。
fn metadata_mtime_ms(metadata: Option<&std::fs::Metadata>) -> u128 {
    metadata
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

pub(super) fn trace_sql<'a>(
    trace: &'a Option<StorageTraceContext>,
    operation: &'static str,
    table: Option<&str>,
    statement: &str,
    params: &[Value],
) -> Option<HostSpanScope> {
    let context = trace.as_ref()?;
    let mut attrs = vec![
        ("db.system", "sqlite".to_string()),
        ("db.operation", operation.to_string()),
        ("db.statement", bounded_attr(statement)),
        ("db.parameters", sql_params_attr(table, params)),
    ];
    if let Some(table) = table {
        attrs.push(("db.sql.table", table.to_string()));
    }
    Some(context.runtime.span_with_attributes(
        "helix.storage.sqlite.query",
        TraceDirection::Internal,
        context.carrier.as_ref(),
        attrs,
    ))
}

fn sql_params_json(params: &[Value]) -> String {
    let json_values: Vec<_> = params
        .iter()
        .map(|value| match value {
            Value::Null => json!(null),
            Value::Integer(v) => json!(v),
            Value::Real(v) => json!(v),
            Value::Text(v) => json!(v),
            Value::Blob(v) => json!({
                "type": "blob",
                "bytes": v.len(),
                "base64": bounded_attr(&base64_encode(v)),
            }),
        })
        .collect();
    serde_json::to_string(&json_values).unwrap_or_else(|_| "[]".to_string())
}

fn sql_params_attr(table: Option<&str>, params: &[Value]) -> String {
    if table == Some("pending_media") {
        return "[REDACTED]".to_string();
    }
    bounded_attr(&sql_params_json(params))
}

fn bounded_attr(value: &str) -> String {
    if value.len() <= SQL_TRACE_MAX_BYTES {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= SQL_TRACE_MAX_BYTES)
        .last()
        .unwrap_or(0);
    format!(
        "{}...[truncated {} bytes]",
        &value[..boundary],
        value.len() - boundary
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_media_parameters_are_redacted_while_regular_tables_remain_observable() {
        let params = vec![
            Value::Text("/tmp/private.pdf".to_string()),
            Value::Text("https://signed.example/put?token=secret".to_string()),
            Value::Text("Authorization: secret".to_string()),
        ];

        let redacted = sql_params_attr(Some("pending_media"), &params);
        assert_eq!(redacted, "[REDACTED]");
        assert!(!redacted.contains("private.pdf"));
        assert!(!redacted.contains("token=secret"));

        let regular = sql_params_attr(Some("message"), &params);
        assert!(regular.contains("private.pdf"));
        assert!(regular.contains("token=secret"));
    }
}
