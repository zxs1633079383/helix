//! 洪峰压测：engine_loop 端到端泵吞吐（Persist → PortReply 回灌闭环）
//!
//! ## 测什么
//!
//! 灌 10_000 个 Tick::Command，每个 Command 让 PumpModule 吐 1 个
//! Effect::Persist{MonotonicUpsert}，driver 兑现（SQLite 写）后把
//! Tick::PortReply 回灌进 engine，模块计数。
//!
//! ## 测试用例
//!
//! 1. `stress_engine_pump_10k_no_window`（死锁验证）：
//!    裸灌 10,000 个 Command，**无人工限流窗口**，验证 E1 修复后不冻结。
//!    期望：全部 10_000 个 PortReply 在 120s 内完成，吞吐打印到 stderr。
//!
//! 2. `stress_engine_pump_10k_windowed`（对照组）：
//!    保留原有 in-flight window=100 的有背压版本，测对照吞吐。
//!
//! ## 原死锁根因（E1，已修复）
//!
//! 原实现：PortReply 回灌与入站 Tick 共用 cap=256 有界 mpsc，且
//! dispatch_owned_effects 末尾 await 全部 persist handles。洪峰 >256 pending
//! 即环形等待冻结。修复后 PortReply 走独立 unbounded 回灌通道，主循环
//! 不阻塞磁盘，biased select 优先消费回灌。
//!
//! ## 跑法
//!
//! ```bash
//! cargo test --release -p helix-driver-native stress -- --nocapture --test-threads=1
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use helix_core::effect::{Effect, EffectSink, MonotonicUpsertSpec, StorageOp};
use helix_core::error::CoreError;
use helix_core::module_host::Module;
use helix_core::tick::AppCommand;
use helix_core::{ExecutionShell, Tick};

use helix_driver_native::{
    engine_loop::{run_engine_loop, EngineConfig},
    NativeEventSink, NativeHttp, NativeStorage,
};

// ─── PumpModule：每个 Command 吐 1 个 Persist，PortReply 计数 ────────────────

struct PumpModule {
    next_corr: u64,
    replies: Arc<AtomicU64>,
}

impl Module for PumpModule {
    fn name(&self) -> &'static str {
        "stress-pump"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::Command(c) if c.name == "pump")
    }

    fn handle(
        &mut self,
        tick: &Tick,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        match tick {
            Tick::Command(_) => {
                let corr = helix_core::Correlation::from_raw(self.next_corr);
                self.next_corr += 1;
                out.push(Effect::Persist {
                    corr,
                    ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                        scope_key: "stress:pump".to_string(),
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

// ─── 辅助：建存储 + 启引擎 ────────────────────────────────────────────────────

async fn setup_engine(db_path: &str) -> (mpsc::Sender<Tick>, Arc<AtomicU64>) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
    let _ = std::fs::remove_file(format!("{db_path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open temp storage");
    let http = NativeHttp::new().expect("http");
    let (event_sink, _event_rx) = NativeEventSink::new();

    let replies = Arc::new(AtomicU64::new(0));
    let mut shell = ExecutionShell::new();
    shell.register(PumpModule {
        next_corr: 1,
        replies: Arc::clone(&replies),
    });

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let config = EngineConfig { storage, http, event_sink };
    let tick_tx_for_loop = tick_tx.clone();
    tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config).await;
    });

    (tick_tx, replies)
}

// ─── 测试 1：裸灌 10k，无限流窗口（E1 死锁验证正向断言）────────────────────

/// E1 修复后，裸灌 10,000 个 Command（无人工背压窗口）必须全部闭环完成不冻结。
///
/// 修复前此测试必然超时（复现死锁）；修复后期望在 120s 内收到全部 10k PortReply。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_engine_pump_10k_no_window() {
    const N: u64 = 10_000;
    let (tick_tx, replies) = setup_engine("/tmp/helix_stress_engine_nowindow.db").await;

    let start = Instant::now();
    // 裸灌全部 10k Command，不设 in-flight 限制
    for _ in 0..N {
        tick_tx
            .send(Tick::Command(AppCommand::new("pump", &b"{}"[..])))
            .await
            .expect("send command");
    }
    let send_done = start.elapsed();

    // 轮询计数器直到 10k 回灌完成（上限 120s）
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let got = replies.load(Ordering::Relaxed);
        if got >= N {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "E1 deadlock regression: only {got}/{N} PortReplies after 120s"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let elapsed = start.elapsed();

    eprintln!("\n[stress_engine_pump_10k_no_window] (E1 修复验证，无背压窗口)");
    eprintln!("  persist effects  : {}", N);
    eprintln!("  send phase       : {:.3} ms (无限流，立即全灌)", send_done.as_secs_f64() * 1e3);
    eprintln!("  total roundtrip  : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!("  loop throughput  : {:.0} persist-roundtrips/s", N as f64 / elapsed.as_secs_f64());
    eprintln!("  per-roundtrip    : {:.3} ms", elapsed.as_secs_f64() * 1e3 / N as f64);
    drop(tick_tx);
}

// ─── 测试 2：window=100 背压对照组 ───────────────────────────────────────────

/// 保留原有 in-flight=100 有背压版本，作为吞吐对照参考。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_engine_pump_10k_windowed() {
    const N: u64 = 10_000;
    const WINDOW: u64 = 100;
    let (tick_tx, replies) = setup_engine("/tmp/helix_stress_engine_windowed.db").await;

    let start = Instant::now();
    for i in 0..N {
        while i.saturating_sub(replies.load(Ordering::Relaxed)) >= WINDOW {
            tokio::time::sleep(Duration::from_micros(200)).await;
        }
        tick_tx
            .send(Tick::Command(AppCommand::new("pump", &b"{}"[..])))
            .await
            .expect("send command");
    }
    let send_done = start.elapsed();

    // 轮询计数器直到 10k 回灌完成（上限 120s）
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let got = replies.load(Ordering::Relaxed);
        if got >= N {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timeout: only {got}/{N} PortReplies after 120s"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let elapsed = start.elapsed();

    eprintln!("\n[stress_engine_pump_10k_windowed] (window=100 背压对照)");
    eprintln!("  persist effects  : {}", N);
    eprintln!("  send phase       : {:.3} ms (背压后全部入队)", send_done.as_secs_f64() * 1e3);
    eprintln!("  total roundtrip  : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!("  loop throughput  : {:.0} persist-roundtrips/s", N as f64 / elapsed.as_secs_f64());
    eprintln!("  per-roundtrip    : {:.3} ms", elapsed.as_secs_f64() * 1e3 / N as f64);
    drop(tick_tx);
}
