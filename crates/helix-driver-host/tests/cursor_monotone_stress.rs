//! P1-C：sync-v2 多 channel 并发 cursor 单调性（spec §5.2 场景 C）
//!
//! ## 测什么
//!
//! 两阶段：
//!
//! **阶段 A — 并发正向推进**
//! 100 个 channel 各自独立 task 顺序推进 100 次 monotonic_upsert（scope_key =
//! `channel:{id}`），全部并发同时跑。所有 task 完成后，对每个 channel 读
//! `_helix_monotonic` 表验证终态 value == 100。
//!
//! **阶段 B — 回退尝试（MAX guard 证伪核心）**
//! 终态 value==100 确立之后，对每个 channel 发一次 value=50 的 monotonic_upsert，
//! 断言读回来仍 == 100（`WHERE excluded.value > value` guard 生效，小值写是 no-op）。
//! 若 guard 被破坏（改成无条件 UPDATE），此阶段读回 50，断言 FAIL。
//!
//! ## 判据（与 spec §5.2 C 对应）
//!
//! - ✅ 每个 channel 阶段 A 终态 value == 100（并发写全部落地）
//! - ✅ 无死锁（`CHANNELS × STEPS = 10000` 次写在 60s 内完成）
//! - ✅ 阶段 B 写回退值 50 后 cursor 仍 == 100（MAX guard 真正防回退）
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
/// 两阶段：
/// - 阶段 A：并发顺序正向推进（证明「无死锁 + 全部写落地」）
/// - 阶段 B：对每个 channel 再发一次 value=50 的回退尝试，断言 cursor 仍 == 100
///           （证明 MAX guard 真正防回退，而非依赖「写序恰好递增」）
///
/// 证伪协议：将 storage.rs 的 monotonic_upsert SQL 由
///   `WHERE excluded.value > _helix_monotonic.value`
/// 改成无条件 `DO UPDATE SET value = excluded.value`，
/// 阶段 B 的断言必然 FAIL（读回 50 而非 100）。
///
/// 回归锚: HX-C005 热路径 O(1) + E5 MAX guard 联合保证。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_channel_cursor_monotone_under_concurrency() {
    const CHANNELS: usize = 100;
    const STEPS: usize = 100;
    const REGRESSION_VALUE: i64 = 50; // 阶段 B 故意写一个小于 STEPS 的值

    let (storage, _path) = file_storage_for_cursor().await;

    // ── 阶段 A：100 channel × 100 顺序递增 monotonic_upsert，全部 task 并发启动 ──
    let mut handles = Vec::with_capacity(CHANNELS);
    for ch in 0..CHANNELS {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            for step in 1..=(STEPS as i64) {
                s.monotonic_upsert(MonotonicUpsertSpec {
                    table: "_helix_monotonic",
                    key_col: "scope_key",
                    value_col: "value",
                    touch_col: None,
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

    // 阶段 A 终态验证：每个 channel cursor == STEPS（100）
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
            "阶段 A: channel:{ch} cursor 终态应为 {STEPS}，实际: {val:?}"
        );
    }

    // ── 阶段 B：对每个 channel 发一次回退尝试（value=50 < 当前 100）──────────────
    // MAX guard 语义：`WHERE excluded.value > current_value`
    // → value=50 < 100，不满足条件，SQL 变成无操作（0 行更新），cursor 保持 100。
    // 若 guard 被移除（改成无条件 UPDATE），value 会被覆写成 50，下面断言 FAIL。
    for ch in 0..CHANNELS {
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                value_col: "value",
                touch_col: None,
                scope_key: format!("channel:{ch}"),
                value: REGRESSION_VALUE, // 故意写小于 STEPS 的值
            })
            .await
            .expect("regression monotonic_upsert");
    }

    for ch in 0..CHANNELS {
        let row = storage
            .get(GetSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                key_val: SqlValue::Text(format!("channel:{ch}")),
            })
            .await
            .expect("get monotonic after regression attempt");

        let val = row
            .as_ref()
            .and_then(|r| r.iter().find(|(k, _)| k == "value"))
            .map(|(_, v)| v.clone());

        assert!(
            matches!(val, Some(SqlValue::Integer(v)) if v == STEPS as i64),
            "阶段 B（MAX guard 证伪）: channel:{ch} 写回退值 {REGRESSION_VALUE} 后 cursor 应仍为 {STEPS}，\
             实际: {val:?}（= MAX guard 失效，cursor 被小值覆盖 = 回退）"
        );
    }

    eprintln!(
        "\n[multi_channel_cursor_monotone_under_concurrency] PASS: \
         阶段A {CHANNELS}ch×{STEPS}steps={} writes all@{STEPS}; \
         阶段B 写回退值{REGRESSION_VALUE}后 cursor 仍={STEPS}（MAX guard 生效）",
        CHANNELS * STEPS
    );
}
