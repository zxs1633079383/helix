//! P1-C：sync-v2 多 channel 并发 cursor 单调性（spec §5.2 场景 C）
//!
//! ## 测什么
//!
//! 100 个 channel 各自独立 task 顺序推进 100 次 monotonic_upsert（scope_key =
//! `channel:{id}`），全部并发同时跑。所有 task 完成后，对每个 channel 读
//! `_helix_monotonic` 表验证终态 value == 100。
//!
//! ## 判据（与 spec §5.2 C 对应）
//!
//! - ✅ 每个 channel 的 value == 100（MAX guard 确保不回退）
//! - ✅ 无死锁（`CHANNELS × STEPS = 10000` 次写在 60s 内完成）
//! - ✅ 无 cursor 回退（N=1 写序 + MAX guard 联合保证）
//!
//! ## 为什么放 helix-driver-host/tests
//!
//! monotonic_upsert 是 HostStorage 的共享核实现（两端复用）；cursor 单调性是
//! host driver 的 E5 不变量，与 native/ffi 无关。放 host 测试，native 无需重复。
//!
//! 跑法：
//! ```bash
//! cargo test -p helix-driver-host cursor_monotone -- --nocapture
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use helix_core::effect::{GetSpec, MonotonicUpsertSpec, SqlValue};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

async fn file_storage_for_cursor() -> (HostStorage, String) {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = format!(
        "/tmp/helix_host_cursor_mono_{}_{}.db",
        std::process::id(),
        seq
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let storage = HostStorage::open_sqlite_url(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open file storage for cursor stress");
    (storage, path)
}

/// P1-C：100 channel × 100 monotonic_upsert 并发，验证每 channel cursor 终态单调。
///
/// 回归锚: HX-C005 热路径 O(1) + E5 MAX guard 联合保证。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_channel_cursor_monotone_under_concurrency() {
    const CHANNELS: usize = 100;
    const STEPS: usize = 100;

    let (storage, _path) = file_storage_for_cursor().await;

    // 100 channel × 100 顺序递增 monotonic_upsert，全部 task 并发启动
    let mut handles = Vec::with_capacity(CHANNELS);
    for ch in 0..CHANNELS {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            for step in 1..=(STEPS as i64) {
                s.monotonic_upsert(MonotonicUpsertSpec {
                    scope_key: format!("channel:{ch}"),
                    value: step,
                })
                .await
                .expect("monotonic_upsert");
            }
        }));
    }

    // 等待全部完成，设 60s 超时防死锁
    let all_done = futures_util::future::join_all(handles);
    tokio::time::timeout(Duration::from_secs(60), all_done)
        .await
        .expect("all tasks must complete within 60s (no deadlock)")
        .into_iter()
        .enumerate()
        .for_each(|(i, r)| {
            r.unwrap_or_else(|e| panic!("channel {i} task panicked: {e}"));
        });

    // 验证每个 channel cursor 终态 == STEPS（100）
    for ch in 0..CHANNELS {
        let row = storage
            .get(GetSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                key_val: SqlValue::Text(format!("channel:{ch}")),
            })
            .await
            .expect("get monotonic for channel");

        let val = row
            .as_ref()
            .and_then(|r| r.iter().find(|(k, _)| k == "value"))
            .map(|(_, v)| v.clone());

        assert!(
            matches!(val, Some(SqlValue::Integer(v)) if v == STEPS as i64),
            "channel:{ch} cursor 终态应为 {STEPS}，实际: {val:?}（MAX guard 失败 = cursor 回退）"
        );
    }

    eprintln!(
        "\n[multi_channel_cursor_monotone_under_concurrency] PASS: \
         {CHANNELS} channels × {STEPS} steps = {} writes, all cursors at {STEPS}",
        CHANNELS * STEPS
    );
}
