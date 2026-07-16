use std::future::Future;

use rusqlite::types::Value;
use serde_json::json;

use crate::base64_encode;
use crate::otel::{HostOtelRuntime, HostSpanScope, TraceDirection};
use crate::trace::TraceCarrier;

const SQL_TRACE_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct StorageTraceContext {
    runtime: HostOtelRuntime,
    carrier: Option<TraceCarrier>,
}

impl StorageTraceContext {
    pub fn new(runtime: HostOtelRuntime, carrier: Option<TraceCarrier>) -> Self {
        Self { runtime, carrier }
    }
}

tokio::task_local! {
    static STORAGE_TRACE_CONTEXT: StorageTraceContext;
}

pub async fn with_storage_trace_context<F>(context: StorageTraceContext, future: F) -> F::Output
where
    F: Future,
{
    STORAGE_TRACE_CONTEXT.scope(context, future).await
}

pub(super) fn current_storage_trace_context() -> Option<StorageTraceContext> {
    STORAGE_TRACE_CONTEXT.try_with(Clone::clone).ok()
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
