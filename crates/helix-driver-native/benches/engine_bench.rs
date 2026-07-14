//! criterion 基准：engine_loop 事件泵全链路 roundtrip 吞吐。
//!
//! ## 测什么（与 tests/stress_engine.rs::stress_engine_pump_10k 同构 workload）
//!
//! 灌 N 个 `Tick::Command{name="pump"}` → PumpModule `handle` 吐 1 个
//! `Effect::Persist{MonotonicUpsert}` → driver 分流到 per-table 写 worker（SQLite 写）
//! → 兑现后 `Tick::PortReply` 经独立 unbounded reply_tx 回灌进 engine → 模块计数。
//! 一个 Command 走完「step → Effect → worker → PortReply 回灌」即一个 roundtrip。
//!
//! 报：per-roundtrip 延迟（criterion median）+ roundtrips/s（Throughput::Elements(N)）。
//!
//! ## 与 stress_engine.rs 的关系
//!
//! stress_engine.rs 是 N=1 墙钟 + 正确性断言（E1 死锁 / graceful drain 回归锚），无统计量。
//! 本 bench 补齐**统计学严格**的热路径覆盖：warmup + 多 sample → median + CI，可签回归。
//! 这里只测**裸灌无背压窗口**形态（对应 stress_engine_pump_10k_no_window，E1 修复后路径）。
//!
//! ## 方法学铁律（本文件遵守）
//!
//! - tokio multi_thread Runtime（与 stress test `worker_threads=4` 同构，泵需并发 worker）
//! - `iter_batched`：engine + storage 搭建放 setup 闭包（**不计入测量**），独立临时库；
//!   measured routine 只含「灌 N 个 Command + 轮询 N 个 PortReply 回灌完成」的泵 roundtrip
//! - warmup ≥ 3s + 充足 sample（sample_size 调低，因单次 routine 跑 N 个 roundtrip 较重）
//! - SmallInput：每次 iter 独立建库 + 独立 engine task，互不污染
//!
//! ## 回归判据
//!
//! 不硬编绝对耗时基线（WAL/prepare_cached/机器差异会漂）。回归以 criterion 自身
//! median 对比（`target/criterion/engine_loop/.../base`）为准：
//! **per-roundtrip median 回归 > 2x 判红**（典型回归根因：E1 类泵死锁/背压退化、
//! 写路径 read-modify-write 退化、回灌通道误改为有界堵塞）。
//!
//! 跑法：`cargo bench -p helix-driver-native --bench engine_bench`
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use tokio::sync::{mpsc, oneshot};

use helix_core::effect::{Effect, EffectSink, MonotonicUpsertSpec, StorageOp};
use helix_core::error::CoreError;
use helix_core::module_host::Module;
use helix_core::tick::AppCommand;
use helix_core::{ExecutionShell, Tick};

use helix_driver_native::{
    engine_loop::{run_engine_loop, EngineConfig},
    HostNetworkConfig, NativeEventSink, NativeHttp, NativeStorage,
};

/// 每个 roundtrip 的命令数：对齐 stress test 的 10k 量级则单次 routine 太重（bench 跑不完），
/// 取 1_000 —— 仍足够压满 job queue（cap=1024）触发真实背压/回灌闭环，且单 routine ~ms 级。
const ROUNDTRIPS: u64 = 1_000;

// ─── PumpModule：每个 Command 吐 1 个 Persist，PortReply 计数（与 stress test 同构）──────

struct PumpModule {
    next_corr: u64,
    replies: Arc<AtomicU64>,
}

impl Module for PumpModule {
    fn name(&self) -> &'static str {
        "bench-pump"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::Command(c) if c.name == "pump")
    }

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        match tick {
            Tick::Command(_) => {
                let corr = helix_core::Correlation::from_raw(self.next_corr);
                self.next_corr += 1;
                out.push(Effect::Persist {
                    corr,
                    ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                        table: "_helix_monotonic",
                        key_col: "scope_key",
                        value_col: "value",
                        touch_col: None,
                        scope_key: "bench:pump".to_string(),
                        value: corr.raw() as i64,
                    })],
                });
            }
            Tick::PortReply { .. } => {
                self.replies.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_start(&mut self, _out: &mut EffectSink) -> Result<(), CoreError> {
        Ok(())
    }
}

/// setup 闭包用：建独立临时库 + 启引擎 task，返回灌入端 + 回灌计数器 + 关停句柄。
/// **此函数耗时不计入 measured routine**（criterion iter_batched 的 setup 阶段）。
async fn setup_engine() -> (
    mpsc::Sender<Tick>,
    Arc<AtomicU64>,
    tokio::task::JoinHandle<()>,
    oneshot::Sender<()>,
) {
    // 每次 setup 独立库：进程 id + 单调序号，互不污染
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_path = format!("/tmp/helix_bench_engine_{}_{}.db", std::process::id(), n);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
    let _ = std::fs::remove_file(format!("{db_path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open temp storage");
    let http =
        NativeHttp::with_config(HostNetworkConfig::new("http://127.0.0.1", "ws://127.0.0.1"))
            .expect("http");
    let (event_sink, _event_rx) = NativeEventSink::new();

    let replies = Arc::new(AtomicU64::new(0));
    let mut shell = ExecutionShell::new();
    shell.register(PumpModule {
        next_corr: 1,
        replies: Arc::clone(&replies),
    });

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let config = EngineConfig {
        storage,
        http,
        uploader: helix_driver_host::SharedFileUploader::default(),
        event_sink,
        metrics: Arc::new(helix_driver_host::NoopMetricSink),
        max_http_inflight: 8,
    };
    let tick_tx_for_loop = tick_tx.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config, shutdown_rx).await;
    });

    (tick_tx, replies, handle, shutdown_tx)
}

/// measured routine：裸灌 N 个 Command + 轮询直到 N 个 PortReply 全部回灌完成。
/// 只含泵 roundtrip 本身——搭建 + 关停清理都在 routine 之外。
async fn run_pump_roundtrips(tick_tx: &mpsc::Sender<Tick>, replies: &AtomicU64) {
    // 边界输入零信任：send 失败说明 engine task 已退（不该在测量期发生）→ 显式 expect 标识异常
    for _ in 0..ROUNDTRIPS {
        tick_tx
            .send(Tick::Command(AppCommand::new("pump", &b"{}"[..])))
            .await
            .expect("bench send command (engine task alive)");
    }
    // 轮询回灌计数器直至闭环完成（带兜底 deadline 防 E1 类死锁回归把 bench 挂死）
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if replies.load(Ordering::Relaxed) >= ROUNDTRIPS {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine pump regression: roundtrips did not close within 30s (deadlock?)"
        );
        tokio::time::sleep(Duration::from_micros(200)).await;
    }
}

fn bench_engine_pump(c: &mut Criterion) {
    // 泵依赖并发写 worker → multi_thread runtime（与 stress test worker_threads=4 同构）
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("multi-thread runtime");

    let mut group = c.benchmark_group("engine_loop");
    group.throughput(Throughput::Elements(ROUNDTRIPS));
    group.warm_up_time(Duration::from_secs(3)); // 铁律：warmup ≥ 3s
    group.measurement_time(Duration::from_secs(20)); // 单 routine 较重 → 拉长测量窗口取足 sample
    group.sample_size(20); // 充足但不过量（每 sample 跑 ROUNDTRIPS 个 roundtrip）

    group.bench_function("pump_roundtrip_1k", |b| {
        // iter_batched：setup（建库 + 启 engine）不计时；routine 只测泵 roundtrip。
        // 每个 sample 用独立库 + 独立 engine task → 互不污染、可重复。
        //
        // criterion 对 routine 闭包**整体计时**，所以：
        // - setup 闭包：建库 + 启 engine（不计时）
        // - routine 闭包：只跑泵 roundtrip，**把关停句柄原样 return** → 关停（drop senders +
        //   join engine task）发生在 criterion 计时窗口**之外**（iter_batched 在计时停止后
        //   才 drop routine 返回值），不污染 per-roundtrip median。
        b.iter_batched(
            || rt.block_on(setup_engine()),
            |(tick_tx, replies, handle, shutdown_tx)| {
                rt.block_on(run_pump_roundtrips(&tick_tx, &replies));
                // return 关停句柄 → 不在测量窗口内 drop/join
                (tick_tx, handle, shutdown_tx)
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_engine_pump);
criterion_main!(benches);
