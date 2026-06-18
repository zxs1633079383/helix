//! P3 基准：current_thread vs multi_thread runtime 纯 storage 层吞吐对照
//!
//! ## 测什么
//!
//! 同一套 HostStorage 操作（monotonic_upsert + batch_upsert），在两种 tokio runtime
//! 形态下各跑 criterion 基准，断言「纯 storage 层吞吐差 <15%」（spec §3 / spec §5.2 场景 F）。
//!
//! - **multi_thread（native 形态）**：默认 CPU 核数 worker 线程 + blocking pool
//! - **current_thread（FFI 形态）**：单 worker 线程 + blocking pool（`enable_all()`）
//!
//! ## 为什么差异应 <15%
//!
//! storage ops 走 `spawn_blocking` → 卸到独立 blocking pool。blocking pool 大小
//! 不受 `current_thread` 单 worker 限制（tokio spec §3）。纯 storage 吞吐几乎
//! 全被 rusqlite I/O 决定，runtime 线程模型差异在纯 I/O 路径上可忽略。
//!
//! ## 判据（替代「FFI pump stress」，因 helix_query 仍是 stub）
//!
//! criterion 报出的 **median**：
//!   - `current_thread` median / `multi_thread` median ≤ 1.15（差 ≤15%）
//!   - 若 > 1.15 → 说明 current_thread 的 blocking pool 或 enable_all 有问题，须排查
//!
//! ## 方法学
//!
//! - 文件 WAL 库（贴生产，不用 `:memory:`）；库 + 连接只建一次（计时区外）。
//! - 每个 runtime 形态用独立库文件（避免 WAL 锁干扰）。
//! - `Throughput::Elements(N)` 报 calls/s。
//! - 阈值 15% 写在注释而非 assert（criterion 不提供跨 group 比较断言），
//!   数字靠人工 review criterion 报告；CI 会在 median 回归 >2x 时自动标 `regressed`。
//!
//! 跑法：
//! ```bash
//! cargo bench -p helix-driver-host --bench runtime_compare -- --nocapture
//! ```

#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use helix_core::effect::{MonotonicUpsertSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;

static DB_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDb {
    path: String,
    url: String,
}

impl TempDb {
    fn new(label: &str) -> Self {
        let n = DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = format!(
            "{}/helix_rt_compare_{}_{}_{}.db",
            std::env::temp_dir().display(),
            label,
            std::process::id(),
            n
        );
        let url = format!("sqlite://{path}?mode=rwc");
        Self { path, url }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(format!("{}-wal", self.path));
        let _ = std::fs::remove_file(format!("{}-shm", self.path));
    }
}

/// 在给定 runtime 上打开文件 WAL 库（建表在外，不在计时区）。
fn open_storage(rt: &tokio::runtime::Runtime, url: &str) -> HostStorage {
    rt.block_on(async {
        let s = HostStorage::open_sqlite_url(url)
            .await
            .expect("open storage");
        s.execute_raw(
            "CREATE TABLE IF NOT EXISTS rt_bench \
             (seq INTEGER PRIMARY KEY, msg_id TEXT, raw TEXT)",
        )
        .await
        .expect("create rt_bench table");
        s
    })
}

/// 生成 N 行测试数据
fn make_rows(n: u64) -> Vec<Vec<(String, SqlValue)>> {
    (1..=n)
        .map(|s| {
            vec![
                ("seq".to_string(), SqlValue::Integer(s as i64)),
                ("msg_id".to_string(), SqlValue::Text(format!("msg_{s}"))),
                ("raw".to_string(), SqlValue::Text("x".repeat(100))),
            ]
        })
        .collect()
}

/// P3-A：monotonic_upsert (cursor 热路径) — current_thread vs multi_thread
///
/// 判据：`current_thread` median ≤ `multi_thread` median × 1.15
fn bench_monotonic_upsert_runtime_compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime_compare/monotonic_upsert");
    group.throughput(Throughput::Elements(1));

    // ── multi_thread runtime（native 形态）──
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("multi_thread rt");
        let db = TempDb::new("multi");
        let storage = open_storage(&rt, &db.url);
        let counter = std::cell::Cell::new(1i64);

        group.bench_with_input(
            BenchmarkId::new("monotonic_upsert", "multi_thread"),
            &"multi_thread",
            |b, _| {
                b.iter(|| {
                    let v = counter.get();
                    counter.set(v + 1);
                    rt.block_on(storage.monotonic_upsert(MonotonicUpsertSpec {
                        scope_key: "rt:cursor".to_string(),
                        value: v,
                    }))
                    .expect("monotonic multi");
                });
            },
        );
    }

    // ── current_thread runtime（FFI 形态）──
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread rt");
        let db = TempDb::new("current");
        let storage = open_storage(&rt, &db.url);
        let counter = std::cell::Cell::new(1i64);

        group.bench_with_input(
            BenchmarkId::new("monotonic_upsert", "current_thread"),
            &"current_thread",
            |b, _| {
                b.iter(|| {
                    let v = counter.get();
                    counter.set(v + 1);
                    rt.block_on(storage.monotonic_upsert(MonotonicUpsertSpec {
                        scope_key: "rt:cursor".to_string(),
                        value: v,
                    }))
                    .expect("monotonic current");
                });
            },
        );
    }

    group.finish();
}

/// P3-B：batch_upsert 100 行 — current_thread vs multi_thread
///
/// 判据：`current_thread` median ≤ `multi_thread` median × 1.15
fn bench_batch_upsert_runtime_compare(c: &mut Criterion) {
    const N: u64 = 100;
    let mut group = c.benchmark_group("runtime_compare/batch_upsert_100");
    group.throughput(Throughput::Elements(N));
    let template = make_rows(N);

    // ── multi_thread ──
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("multi_thread rt");
        let db = TempDb::new("batch_multi");
        let storage = open_storage(&rt, &db.url);
        let rows = template.clone();

        group.bench_with_input(
            BenchmarkId::new("batch_upsert_100", "multi_thread"),
            &"multi_thread",
            |b, _| {
                b.iter(|| {
                    rt.block_on(storage.batch_upsert(UpsertSpec {
                        table: "rt_bench",
                        rows: rows.clone(),
                        conflict_key: Some("seq"),
                    }))
                    .expect("batch multi");
                });
            },
        );
    }

    // ── current_thread ──
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread rt");
        let db = TempDb::new("batch_current");
        let storage = open_storage(&rt, &db.url);
        let rows = template.clone();

        group.bench_with_input(
            BenchmarkId::new("batch_upsert_100", "current_thread"),
            &"current_thread",
            |b, _| {
                b.iter(|| {
                    rt.block_on(storage.batch_upsert(UpsertSpec {
                        table: "rt_bench",
                        rows: rows.clone(),
                        conflict_key: Some("seq"),
                    }))
                    .expect("batch current");
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_monotonic_upsert_runtime_compare,
    bench_batch_upsert_runtime_compare,
);
criterion_main!(benches);
