//! 洪峰压测：engine_loop 端到端泵吞吐（Persist → PortReply 回灌闭环）
//!
//! ## 测什么
//!
//! 灌 10_000 个 Tick::Command，每个 Command 让 PumpModule 吐 1 个
//! Effect::Persist{MonotonicUpsert}，driver 兑现（SQLite 写）后把
//! Tick::PortReply 回灌进 engine，模块计数。度量从第一个 Command 发出到
//! 第 10_000 个 PortReply 被模块收到的总耗时 = 完整闭环吞吐：
//!
//! ```text
//! Command → step → Persist effect → SQLite 写（autocommit）→ PortReply
//!         → mpsc 回灌 → step → 计数
//! ```
//!
//! ## ⚠️ 实测发现的死锁（2026-06-11，洪峰压测第一轮）
//!
//! **裸灌 > 256（channel 容量）个 Command 会让 engine_loop 永久死锁**：
//!
//! 1. 测试侧把 mpsc(cap=256) 灌满后继续 send → 阻塞等容量
//! 2. engine_loop recv 1 个 Command → step → dispatch_owned_effects
//!    → `h.await` 等 persist 任务完成
//! 3. persist 任务写完 SQLite 后 `tx_clone.send(Tick::PortReply).await`
//!    → channel 满 → 排队等容量
//! 4. 容量只能靠 engine_loop recv 释放，而 engine_loop 卡在 2 → **环形等待**
//!
//! 实测：monotonic 表 value 停在 1（第一个 persist 完成后全链路冻结）。
//! 根因：PortReply 回灌与入站 Tick **共用同一条有界 mpsc**，且 dispatch
//! 同步 await persist 完成。修复方向（任选其一）：
//! - PortReply 走独立 unbounded/专用 channel（回灌优先级高于入站）
//! - engine_loop 不 await persist handle（fire-and-forget + 顺序保证另行设计）
//! - 入站侧（Transport/Command 注入点）实施背压上限 < channel 容量
//!
//! 本测试因此用 **bounded in-flight window（100 < 256）** 闭环测吞吐 ——
//! 这也是真实 driver 接入时必须遵守的入站背压契约。
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

// ─── 测试：1 万 Persist effect 回灌闭环 ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_engine_pump_10k_persist_roundtrip() {
    const N: u64 = 10_000;
    let path = "/tmp/helix_stress_engine.db";
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
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
    let _loop_handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config).await;
    });

    // 灌 10k Command —— 必须限制 in-flight < channel 容量（见文件顶死锁注释）。
    // window=100：未回灌的 Command ≤ 100，channel 内 Command+PortReply ≤ 200 < 256。
    const WINDOW: u64 = 100;
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

    eprintln!("\n[stress_engine_pump_10k_persist_roundtrip]");
    eprintln!("  persist effects  : {}", N);
    eprintln!("  send phase       : {:.3} ms (背压后全部入队)", send_done.as_secs_f64() * 1e3);
    eprintln!("  total roundtrip  : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!("  loop throughput  : {:.0} persist-roundtrips/s", N as f64 / elapsed.as_secs_f64());
    eprintln!("  per-roundtrip    : {:.3} ms", elapsed.as_secs_f64() * 1e3 / N as f64);
    drop(tick_tx);
}
