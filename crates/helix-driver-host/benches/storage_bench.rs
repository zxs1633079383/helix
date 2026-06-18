//! criterion 严格基准：HostStorage 写热路径吞吐。
//!
//! 落实 ROADMAP P2 + stress-verify(2026-06-15 REJECT)的「N=1 墙钟 → criterion」整改：
//! 旧法只跑单次 `Instant::now()` 差值，无 warmup、无多迭代、无统计分布，得到的数字
//! 不可签（单点噪声 / 冷缓存 / GC 抖动全混在一个样本里）。本文件用 criterion 提供
//! warmup（≥3s 默认）+ 多 sample 迭代 + 自动统计：**criterion 报 median + MAD（中位
//! 绝对偏差）+ 各 percentile + Tukey 离群分析**，给可签的硬数字。
//!
//! ## 方法学（满足 stress-verify 审计）
//! - 全部用**文件 WAL 库**（`mode=rwc`），不用 `:memory:`：贴生产形态（WAL 落盘 +
//!   busy_timeout + 单写者），且 HostStorage 只读池只对文件库启用（内存库回退写连接）。
//! - **库 + 连接（1 写 + 4 读，READ_POOL_SIZE=4）只在 `bench_function` 外建一次**，建表
//!   也只跑一次；measured routine（`b.iter`）**只含被测 storage op**——库/连接的 open
//!   与 drop（关 5 个 rusqlite fd + WAL 收尾 + 删库文件）全部落在计时区之外。
//!   旧实现用 `iter_batched` 每迭代新建/销毁库 + 连接，而 criterion 0.5 的 `iter_batched`
//!   在 `measurement.start()..end()` 之间 drop 被 move 进 routine 的 input，于是每次测量
//!   都把「关 5 连接 + 3 次 remove_file」算进去，对 µs 级的 monotonic 直接支配测量、对
//!   batch 也系统性抬高 median——本次整改即消除该污染。
//! - upsert 幂等（ON CONFLICT DO UPDATE 同 PK），重复迭代写同一组行得稳态（行数不增长，
//!   纯写路径吞吐）；monotonic 用进程内单调 value 推进，命中 MAX guard 的 UPDATE 分支
//!   （贴 cursor 前进语义）。
//! - async storage 调用经 tokio `Runtime::block_on` 跑（criterion 自身同步驱动）。
//! - 用 `Throughput::Elements` 报 rows/s（批量）或 calls/s（cursor 单条）。
//! - 每个 bench 用独立临时库文件（进程 id + 原子计数器）；`TempDb` Drop 在 bench_function
//!   返回后（计时区外）清理库文件 + WAL/SHM 旁路。
//!
//! ## 可签的回归判据（替代旧的 1s / 10x 松阈值）
//! 以 criterion 报出的 **median** 为准（非 mean，抗离群）。任一 bench 的 median 相对
//! 已存基线（`target/criterion/<bench>/base`）**回归 > 2x 即判红**（criterion 会标
//! `Performance has regressed` 并给变化百分比 + 置信区间）。MAD/percentile 若离散度
//! 异常放大（p99 >> median）说明引入了不稳定路径，同样需排查。
//! 跑法：`cargo bench -p helix-driver-host`（首次建 baseline；后续自动对比）。
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use helix_core::effect::{MonotonicUpsertSpec, Row, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;

/// 进程内单调计数器，保证同进程内多个 bench 各拿到独立库文件名。
static DB_SEQ: AtomicU64 = AtomicU64::new(0);

/// 临时文件 WAL 库句柄：构造时分配唯一路径，Drop 时清理库文件 + WAL/SHM 旁路。
struct TempDb {
    /// 纯文件系统路径（用于清理）。
    path: String,
    /// `sqlite:` URL（用于 `HostStorage::open_sqlite_url`，带 `mode=rwc` 走文件 WAL）。
    url: String,
}

impl TempDb {
    fn new() -> Self {
        let n = DB_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = format!(
            "{}/helix_host_bench_{}_{}.db",
            std::env::temp_dir().display(),
            std::process::id(),
            n
        );
        // mode=rwc：read-write-create 文件库（贴生产；非 :memory:）。
        let url = format!("sqlite://{path}?mode=rwc");
        Self { path, url }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        // 清理库文件 + WAL/SHM 旁路文件；忽略不存在错误（best-effort 清理）。
        // 注意：TempDb 在 bench_function 返回后（计时区外）才 drop——见文件头方法学第 2 点。
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(format!("{}-wal", self.path));
        let _ = std::fs::remove_file(format!("{}-shm", self.path));
    }
}

/// 生成 n 行 bench 数据（贴 IM 消息行形态：seq PK + 文本字段 + ~150B 负载）。
fn make_rows(n: u64) -> Vec<Row> {
    (1..=n)
        .map(|s| {
            vec![
                ("seq".to_string(), SqlValue::Integer(s as i64)),
                ("msg_id".to_string(), SqlValue::Text(format!("msg_{s}"))),
                ("actor_id".to_string(), SqlValue::Text("user_1".to_string())),
                ("raw".to_string(), SqlValue::Text("x".repeat(150))),
            ]
        })
        .collect()
}

/// 打开一个建好 `bench_msgs` 表的 HostStorage（用于 upsert bench 的一次性 setup）。
fn open_with_bench_table(rt: &tokio::runtime::Runtime, db: &TempDb) -> HostStorage {
    rt.block_on(async {
        let s = HostStorage::open_sqlite_url(&db.url)
            .await
            .expect("open host storage");
        s.execute_raw(
            "CREATE TABLE IF NOT EXISTS bench_msgs \
             (seq INTEGER PRIMARY KEY, msg_id TEXT, actor_id TEXT, raw TEXT)",
        )
        .await
        .expect("create bench table");
        s
    })
}

/// bench_batch_upsert_5000：单批 5000 行 upsert（贴 E2 单事务路径）。
///
/// 库 + 连接 + 建表只在 `bench_function` 外建一次；measured routine 只跑一次
/// `batch_upsert`（每迭代 clone 一份 5000 行——clone 是输入准备成本，但相对 5000 行
/// 整批写 SQL 的开销可忽略，且不涉及任何 fd / 文件系统 teardown）。upsert 幂等
/// （ON CONFLICT DO UPDATE 同 PK），重复迭代写同一组行得稳态（行数不增长，纯写吞吐）。
fn bench_batch_upsert_5000(c: &mut Criterion) {
    const N: u64 = 5000;
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let db = TempDb::new();
    let storage = open_with_bench_table(&rt, &db);
    let template = make_rows(N);

    let mut group = c.benchmark_group("host_storage");
    group.throughput(Throughput::Elements(N));
    group.bench_function("batch_upsert_5000", |b| {
        b.iter(|| {
            // measured（计时）：仅一次整批 upsert。clone 输入行不涉及 fd/fs teardown。
            rt.block_on(storage.batch_upsert(UpsertSpec {
                table: "bench_msgs",
                rows: template.clone(),
                conflict_key: Some("seq"),
                exclude_from_update: Vec::new(),
            }))
            .expect("upsert");
        });
    });
    group.finish();
    // storage / db 在此（group.finish 后、函数返回时）drop → 关连接 + 删库文件，计时区外。
}

/// bench_batch_upsert_39x100：39 行 ×100 批（贴生产小批形态）。
///
/// 一次 measured routine = 100 次 `batch_upsert`（每次 39 行），共 3900 行；Throughput
/// 按总行数报，得到「生产小批模式下的有效行吞吐」。库 + 连接 + 100 份批次模板只在
/// `bench_function` 外建一次；每迭代 clone 批次模板（输入准备成本，无 fd/fs teardown）。
fn bench_batch_upsert_39x100(c: &mut Criterion) {
    const ROWS_PER_BATCH: u64 = 39;
    const BATCHES: u64 = 100;
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let db = TempDb::new();
    let storage = open_with_bench_table(&rt, &db);
    // 100 份 39 行批次。各批 seq 用不同基偏移，避免 100 批全打同 PK
    // 退化成「99 次纯 UPDATE」失真——更贴近真实多批写入。
    let template: Vec<Vec<Row>> = (0..BATCHES)
        .map(|b_idx| {
            let base = b_idx * ROWS_PER_BATCH;
            (1..=ROWS_PER_BATCH)
                .map(|s| {
                    let seq = base + s;
                    vec![
                        ("seq".to_string(), SqlValue::Integer(seq as i64)),
                        ("msg_id".to_string(), SqlValue::Text(format!("msg_{seq}"))),
                        ("actor_id".to_string(), SqlValue::Text("user_1".to_string())),
                        ("raw".to_string(), SqlValue::Text("x".repeat(150))),
                    ]
                })
                .collect()
        })
        .collect();

    let mut group = c.benchmark_group("host_storage");
    group.throughput(Throughput::Elements(ROWS_PER_BATCH * BATCHES));
    group.bench_function(BenchmarkId::new("batch_upsert", "39x100"), |b| {
        b.iter(|| {
            rt.block_on(async {
                for rows in template.clone() {
                    storage
                        .batch_upsert(UpsertSpec {
                            table: "bench_msgs",
                            rows,
                            conflict_key: Some("seq"),
                            exclude_from_update: Vec::new(),
                        })
                        .await
                        .expect("upsert");
                }
            });
        });
    });
    group.finish();
    // storage / db 在此 drop → 关连接 + 删库文件，计时区外。
}

/// bench_monotonic_upsert：cursor 热路径（per-message），单条 `monotonic_upsert`。
///
/// 贴最热路径（B1/#2 const SQL 后：固定模板 + prepare_cached）。一次 measured routine =
/// 一条 cursor 推进。库 + 连接（`_helix_monotonic` 表由 migrate() 自动建）只在
/// `bench_function` 外建一次；value 在迭代间单调递增（贴生产 cursor 前进语义；命中 MAX
/// guard 的 UPDATE 分支），所以**绝不**在计时区内开/关库——旧实现每迭代新建库 + 连接
/// 再在 routine 内 drop，对 µs 级单条 UPDATE 直接被 teardown 支配，测的几乎是关 5 连接
/// + 删 3 文件而非写吞吐。
fn bench_monotonic_upsert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    let db = TempDb::new();
    let storage = rt.block_on(async {
        HostStorage::open_sqlite_url(&db.url)
            .await
            .expect("open host storage")
    });
    // value 在迭代间单调递增，命中 MAX guard 的 UPDATE 分支（cursor 前进）。
    let counter = std::cell::Cell::new(1i64);

    let mut group = c.benchmark_group("host_storage");
    group.throughput(Throughput::Elements(1));
    group.bench_function("monotonic_upsert", |b| {
        b.iter(|| {
            let value = counter.get();
            counter.set(value + 1);
            rt.block_on(storage.monotonic_upsert(MonotonicUpsertSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                value_col: "value",
                touch_col: None,
                scope_key: "channel_1".to_string(),
                value,
            }))
            .expect("monotonic upsert");
        });
    });
    group.finish();
    // storage / db 在此 drop → 关连接 + 删库文件，计时区外。
}

criterion_group!(
    benches,
    bench_batch_upsert_5000,
    bench_batch_upsert_39x100,
    bench_monotonic_upsert
);
criterion_main!(benches);
