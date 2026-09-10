//! criterion 基准：batch_upsert 写吞吐（E2 修复的常态化回归监测）。
//!
//! a0595607 重构后 `NativeStorage` 委托 `helix-driver-host::HostStorage`，本 bench 仍可
//! 编译并测 native 侧 facade（含委托开销）。**严格的方法学基准（warmup + iter_batched
//! setup 隔离 + 5000 行 / 39×100 / monotonic 多形态 + 可签回归判据）见
//! `helix-driver-host/benches/storage_bench.rs`**——那里直接测 HostStorage 内核，是
//! 写路径性能的权威落点；本文件保留为 native facade 的轻量冒烟。
//!
//! 基线注释更新（2026-06-15）：旧注释「5000 行 ~40-60ms（82K-122K 行/秒）」与现实
//! 脱节——stress-verify 实测整批 upsert 在 ~ms 量级（WAL + 单事务 + prepare_cached 后
//! 远快于旧估），保留旧数字会误导回归判读。**不在此处硬编绝对耗时基线**，回归以
//! criterion 自身的 median 对比（`target/criterion/.../base`）为准：median 回归 > 2x 判红。
//! 跑法：`cargo bench -p helix-driver-native`
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use helix_core::effect::{Row, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_native::storage::NativeStorage;

fn make_rows(n: u64) -> Vec<Row> {
    (1..=n)
        .map(|s| {
            vec![
                ("seq".to_string(), SqlValue::Integer(s as i64)),
                ("msg_id".to_string(), SqlValue::Text(format!("msg_{s}"))),
                ("actor_id".to_string(), SqlValue::Text("user_1".into())),
                ("raw".to_string(), SqlValue::Text("x".repeat(150))),
            ]
        })
        .collect()
}

/// Measure the existing unguarded message upsert workload.
fn bench_batch_upsert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let db = format!(
        "sqlite:///tmp/helix_bench_{}.db?mode=rwc",
        std::process::id()
    );
    let storage = rt.block_on(async {
        let s = NativeStorage::open(&db).await.expect("open");
        s.execute_raw(
            "CREATE TABLE IF NOT EXISTS bench_msgs \
             (seq INTEGER PRIMARY KEY, msg_id TEXT, actor_id TEXT, raw TEXT)",
        )
        .await
        .expect("create bench table");
        s
    });

    let rows = make_rows(390);
    let mut group = c.benchmark_group("storage");
    group.throughput(Throughput::Elements(390));
    group.bench_function("batch_upsert_390", |b| {
        b.iter(|| {
            rt.block_on(storage.batch_upsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "bench_msgs",
                rows: rows.clone(),
                conflict_key: Some("seq"),
                exclude_from_update: Vec::new(),
            }))
            .expect("upsert");
        });
    });
    group.finish();
}

criterion_group!(benches, bench_batch_upsert);
criterion_main!(benches);
