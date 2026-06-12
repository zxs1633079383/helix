//! criterion 基准：batch_upsert 写吞吐（E2 修复的常态化回归监测）
//!
//! 基线（2026-06-11，事务包裹后）：5000 行 ~40-60ms（82K-122K 行/秒）。
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

fn bench_batch_upsert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let db = format!(
        "sqlite:///tmp/helix_bench_{}.db?mode=rwc",
        std::process::id()
    );
    let storage = rt.block_on(async {
        let s = NativeStorage::open(&db).await.expect("open");
        // 建表（与 stress_storage 同构）
        s.batch_upsert(UpsertSpec {
            table: "bench_msgs",
            rows: make_rows(1),
            conflict_key: Some("seq"),
        })
        .await
        .ok(); // 首次会因表不存在失败
        sqlx_bootstrap(&db, &rt);
        s
    });

    let rows = make_rows(390);
    let mut group = c.benchmark_group("storage");
    group.throughput(Throughput::Elements(390));
    group.bench_function("batch_upsert_390", |b| {
        b.iter(|| {
            rt.block_on(storage.batch_upsert(UpsertSpec {
                table: "bench_msgs",
                rows: rows.clone(),
                conflict_key: Some("seq"),
            }))
            .expect("upsert");
        });
    });
    group.finish();
}

fn sqlx_bootstrap(db: &str, rt: &tokio::runtime::Runtime) {
    rt.block_on(async {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(db)
            .await
            .expect("pool");
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS bench_msgs (seq INTEGER PRIMARY KEY, msg_id TEXT, actor_id TEXT, raw TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create table");
    });
}

criterion_group!(benches, bench_batch_upsert);
criterion_main!(benches);
