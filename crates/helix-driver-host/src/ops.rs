//! Shared StorageOp execution helpers for host drivers.

use bytes::Bytes;
use helix_core::effect::StorageOp;
use helix_core::ports::Storage;
use helix_core::tick::{PortError as CorePortError, PortOutcome, ReplyBytes};
use helix_core::PortError;

use crate::rows_to_reply_bytes;

/// Execute a StorageOp batch in order.
///
/// Any failure short-circuits the remaining ops. Success returns the last
/// meaningful read result (`Get`/`Scan`) as shared rows JSON bytes; writes return
/// empty bytes.
pub async fn execute_storage_ops<S>(storage: &S, ops: &[StorageOp]) -> PortOutcome
where
    S: Storage + ?Sized,
{
    let mut last_ok_bytes = Bytes::new();

    for op in ops {
        match op {
            StorageOp::BatchUpsert(spec) => match storage.batch_upsert(spec.clone()).await {
                Ok(()) => {}
                Err(e) => return PortOutcome::Err(classify_port_error(e)),
            },
            StorageOp::BatchUpdate(spec) => match storage.batch_update(spec.clone()).await {
                Ok(()) => {}
                Err(e) => return PortOutcome::Err(classify_port_error(e)),
            },
            StorageOp::MonotonicUpsert(spec) => {
                match storage.monotonic_upsert(spec.clone()).await {
                    Ok(()) => {}
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
            StorageOp::Get(spec) => match storage.get(spec.clone()).await {
                Ok(Some(row)) => {
                    last_ok_bytes = rows_to_reply_bytes(&[row]);
                }
                Ok(None) => {
                    last_ok_bytes = Bytes::new();
                }
                Err(e) => return PortOutcome::Err(classify_port_error(e)),
            },
            StorageOp::Scan(spec) => match storage.scan(spec.clone()).await {
                Ok(rows) => {
                    last_ok_bytes = rows_to_reply_bytes(&rows);
                }
                Err(e) => return PortOutcome::Err(classify_port_error(e)),
            },
        }
    }

    PortOutcome::Ok(ReplyBytes(last_ok_bytes))
}

/// Compute the per-table-set routing key used by ordered persist workers.
pub fn table_set_key(ops: &[StorageOp]) -> String {
    if ops.is_empty() {
        return String::new();
    }

    let mut tables: Vec<&str> = ops
        .iter()
        .map(|op| match op {
            StorageOp::BatchUpsert(s) => s.table,
            StorageOp::BatchUpdate(s) => s.table,
            StorageOp::MonotonicUpsert(_) => "_helix_monotonic",
            StorageOp::Get(s) => s.table,
            StorageOp::Scan(s) => s.table,
        })
        .collect();
    tables.sort_unstable();
    tables.dedup();
    tables.join("|")
}

/// Convert driver boundary errors into module-visible port errors.
pub fn classify_port_error(e: PortError) -> CorePortError {
    match &e {
        PortError::Storage(_) => CorePortError::Storage(0),
        PortError::Transport(msg) if msg.starts_with("timeout") => CorePortError::Timeout,
        PortError::Transport(_) => CorePortError::Network,
        PortError::Http(msg) => {
            let code = msg
                .split_whitespace()
                .find_map(|w| w.parse::<u16>().ok())
                .unwrap_or(0);
            CorePortError::Http(code)
        }
        _ => CorePortError::Other(0),
    }
}
