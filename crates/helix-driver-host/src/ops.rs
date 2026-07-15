//! Shared StorageOp execution helpers for host drivers.

use bytes::Bytes;
use helix_core::effect::StorageOp;
pub use helix_core::port_codec::classify_port_error;
use helix_core::ports::Storage;
use helix_core::tick::{PortOutcome, ReplyBytes};

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
            StorageOp::GuardedBump(spec) => match storage.guarded_bump(spec.clone()).await {
                Ok(()) => {}
                Err(e) => return PortOutcome::Err(classify_port_error(e)),
            },
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
            StorageOp::BatchDelete(spec) => match storage.batch_delete(spec.clone()).await {
                Ok(()) => {}
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
            StorageOp::MonotonicUpsert(s) => s.table,
            StorageOp::GuardedBump(s) => s.table,
            StorageOp::Get(s) => s.table,
            StorageOp::Scan(s) => s.table,
            StorageOp::BatchDelete(s) => s.table,
        })
        .collect();
    tables.sort_unstable();
    tables.dedup();
    tables.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::StorageOp;
    use helix_core::tick::PortError as CorePortError;
    use helix_core::PortError;

    #[test]
    fn test_classify_port_error_timeout() {
        let e = PortError::Transport("timeout: connection timed out".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Timeout);
    }

    #[test]
    fn test_classify_port_error_network() {
        let e = PortError::Transport("network: connection refused".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Network);
    }

    #[test]
    fn test_classify_port_error_storage() {
        let e = PortError::Storage("disk full".to_string());
        assert!(matches!(classify_port_error(e), CorePortError::Storage(_)));
    }

    #[test]
    fn test_table_set_key_empty() {
        assert_eq!(table_set_key(&[]), "");
    }

    #[test]
    fn test_table_set_key_single_table() {
        use helix_core::effect::UpsertSpec;
        let ops = vec![StorageOp::BatchUpsert(UpsertSpec::new(
            "messages",
            vec![],
            None,
        ))];
        assert_eq!(table_set_key(&ops), "messages");
    }

    #[test]
    fn test_table_set_key_multi_table_sorted() {
        use helix_core::effect::{MonotonicUpsertSpec, UpsertSpec};
        // 两个不同 table 的 ops，顺序无所谓，键必须按字典序排
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec::new("messages", vec![], None)),
            StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                value_col: "value",
                touch_col: None,
                scope_key: "ch:1".to_string(),
                value: 1,
            }),
        ];
        // messages > _helix_monotonic（字典序），键应是 "_helix_monotonic|messages"
        assert_eq!(table_set_key(&ops), "_helix_monotonic|messages");
    }

    #[test]
    fn test_table_set_key_dedup_same_table() {
        use helix_core::effect::UpsertSpec;
        // 同一 table 出现多次，去重后仍是单 key
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec::new("messages", vec![], None)),
            StorageOp::BatchUpsert(UpsertSpec::new("messages", vec![], None)),
        ];
        assert_eq!(table_set_key(&ops), "messages");
    }
}
