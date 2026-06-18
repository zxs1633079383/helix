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
//! 3. `stress_engine_graceful_drain`（shutdown drain 验证，review §3 修复）：
//!    灌 N 个 Command 后 drop tick_tx，等待 run_engine_loop 返回，读 DB 验证
//!    全部 N 条写均已落盘（没有因 detached 任务被丢弃）。
//!
//! ## 原死锁根因（E1，已修复）
//!
//! 原实现：PortReply 回灌与入站 Tick 共用 cap=256 有界 mpsc，且
//! dispatch_owned_effects 末尾 await 全部 persist handles。洪峰 >256 pending
//! 即环形等待冻结。修复后 PortReply 走独立 unbounded 回灌通道，主循环
//! 不阻塞磁盘，biased select 优先消费回灌。
//!
//! ## 背压契约（接入方必须遵守）
//!
//! per-table job queue 容量 = `PERSIST_QUEUE_CAP`（1024）。超限时 dispatch 侧
//! `.await` 阻塞主循环，背压传导到 tick_tx（cap=256）。接入方须使用有界 tick_tx
//! 并在背压下等待（`.send().await`），**不得** `try_send` 丢弃或 `send_timeout` 截断。
//! 违反此契约 → job queue 仍会积压；job queue 是最终背压边界，由 driver 持有。
//!
//! ## 跑法
//!
//! ```bash
//! cargo test --release -p helix-driver-native stress -- --nocapture --test-threads=1
//! ```

// 回归锚: E1（泵死锁）· graceful drain（review 回炉 §3）—— 见 docs/过会方案/helix-架构评审-待办清单.md
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use helix_core::effect::{Effect, EffectSink, MonotonicUpsertSpec, StorageOp};
use helix_core::error::CoreError;
use helix_core::module_host::Module;
use helix_core::tick::AppCommand;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::HostNetworkConfig;

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

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
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

fn make_test_http() -> NativeHttp {
    NativeHttp::with_config(HostNetworkConfig::new("http://127.0.0.1", "ws://127.0.0.1"))
        .expect("http")
}

async fn setup_engine(db_path: &str) -> (mpsc::Sender<Tick>, Arc<AtomicU64>) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
    let _ = std::fs::remove_file(format!("{db_path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open temp storage");
    let http = make_test_http();
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
        event_sink,
        max_http_inflight: 8,
    };
    let tick_tx_for_loop = tick_tx.clone();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config, shutdown_rx).await;
    });

    (tick_tx, replies)
}

/// 与 setup_engine 相同，但返回 storage 句柄、引擎 JoinHandle 和 shutdown sender，
/// 用于 graceful drain 验证。
async fn setup_engine_with_handle(
    db_path: &str,
) -> (
    mpsc::Sender<Tick>,
    Arc<AtomicU64>,
    NativeStorage,
    tokio::task::JoinHandle<()>,
    oneshot::Sender<()>,
) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
    let _ = std::fs::remove_file(format!("{db_path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open temp storage");
    // 独立读句柄用于事后验证（引擎退出后才查）
    let storage_for_verify = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open verify storage");

    let http = make_test_http();
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
        event_sink,
        max_http_inflight: 8,
    };
    let tick_tx_for_loop = tick_tx.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config, shutdown_rx).await;
    });

    (tick_tx, replies, storage_for_verify, handle, shutdown_tx)
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
    eprintln!(
        "  send phase       : {:.3} ms (无限流，立即全灌)",
        send_done.as_secs_f64() * 1e3
    );
    eprintln!("  total roundtrip  : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!(
        "  loop throughput  : {:.0} persist-roundtrips/s",
        N as f64 / elapsed.as_secs_f64()
    );
    eprintln!(
        "  per-roundtrip    : {:.3} ms",
        elapsed.as_secs_f64() * 1e3 / N as f64
    );
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
    eprintln!(
        "  send phase       : {:.3} ms (背压后全部入队)",
        send_done.as_secs_f64() * 1e3
    );
    eprintln!("  total roundtrip  : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!(
        "  loop throughput  : {:.0} persist-roundtrips/s",
        N as f64 / elapsed.as_secs_f64()
    );
    eprintln!(
        "  per-roundtrip    : {:.3} ms",
        elapsed.as_secs_f64() * 1e3 / N as f64
    );
    drop(tick_tx);
}

// ─── 测试 3：graceful drain（shutdown drain 验证，review §3 修复）────────────

/// drop(tick_tx) 触发引擎 graceful drain 后，全部已接收 Persist 必须已落盘。
///
/// 旧实现：tick_rx 关闭 → 立即 break → worker 变 detached → 积压写可能丢弃。
/// 新实现：break 后 drain 全部 worker → join 全部 JoinHandle → 所有写落盘后返回。
///
/// 测试方法：
/// 1. 灌 N 个 Command（每个触发一个 MonotonicUpsert，scope_key="stress:pump"，值递增）
/// 2. 等全部 PortReply 到达（确保 N 个写都被 engine 接收并入队）
/// 3. drop(tick_tx) → 引擎进入 graceful drain
/// 4. await engine_join（等 drain 完成）
/// 5. 查 DB：_helix_monotonic WHERE scope_key="stress:pump" 的 value 必须 = N
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_engine_graceful_drain() {
    use helix_core::effect::GetSpec;
    use helix_core::ports::Storage;

    const N: u64 = 500; // 小批量，快速验证 drain 正确性
    let db_path = "/tmp/helix_stress_drain.db";
    let (tick_tx, replies, verify_storage, engine_join, shutdown_tx) =
        setup_engine_with_handle(db_path).await;

    let start = Instant::now();

    // 灌 N 个 Command，背压等待（保证全部被 engine 接收）
    for _ in 0..N {
        tick_tx
            .send(Tick::Command(AppCommand::new("pump", &b"{}"[..])))
            .await
            .expect("send command");
    }

    // 等全部 PortReply 到达（确保 engine 已经把 N 个 Persist 全部入队给 worker）
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if replies.load(Ordering::Relaxed) >= N {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for PortReplies before drain"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 发送 shutdown 信号 → engine 停止接收新 Tick，进入 graceful drain
    drop(tick_tx); // 避免发 shutdown 后有悬空 tick_tx 继续写
    shutdown_tx.send(()).ok();

    // 等待引擎 drain 完成（run_engine_loop 返回）
    let drain_timeout = tokio::time::timeout(Duration::from_secs(30), engine_join).await;
    assert!(
        drain_timeout.is_ok(),
        "engine graceful drain timed out after 30s"
    );
    assert!(
        drain_timeout.unwrap().is_ok(),
        "engine task panicked during drain"
    );

    let elapsed = start.elapsed();

    // 查 DB：_helix_monotonic 里 scope_key="stress:pump" 的 value 应等于 N
    // (MonotonicUpsert 用 MAX guard，每个 Command 写自己的 corr 值，最终是最大值 = N)
    let row = verify_storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: helix_core::effect::SqlValue::Text("stress:pump".to_string()),
        })
        .await
        .expect("db get after drain");

    assert!(
        row.is_some(),
        "graceful drain regression: _helix_monotonic row missing after drain (all writes discarded?)"
    );
    let row = row.unwrap();
    let val = row.iter().find(|(k, _)| k == "value").map(|(_, v)| v);
    // MonotonicUpsert MAX guard：最终 value = max(所有 corr)；corr 从 1 开始递增，最大值 = N
    assert!(
        matches!(val, Some(helix_core::effect::SqlValue::Integer(v)) if *v >= N as i64),
        "graceful drain regression: value={val:?} expected >= {N}"
    );

    eprintln!("\n[stress_engine_graceful_drain] (shutdown drain 验证)");
    eprintln!("  commands sent    : {}", N);
    eprintln!("  replies received : {}", replies.load(Ordering::Relaxed));
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!("  DB value after drain: {:?}", val);
}

// ─── 测试 4：P2 三入口混合 flood（Persist 写 + Persist 读 + PortReply 回灌）────────
//
// spec §5.2 场景 E：三类入口并发：
//   - 500 个 Tick::Command("pump") → PumpModule → Effect::Persist{MonotonicUpsert}
//   - 100 个 Tick::Command("query") → QueryModule → Effect::Persist{StorageOp::Get}
//   - 所有 PortReply 由 engine 自动回灌（回灌优先，biased select）
//
// 判据：
//   - ✅ 无死锁（120s 内全部完成）
//   - ✅ 全部 500 写 PortReply 到达（PumpModule.replies == 500）
//   - ✅ 全部 100 读 PortReply 到达（QueryModule.replies == 100）
//   - ✅ cursor 最终值 ≥ 500（MAX guard，实际 == 500，因 corr 从 1 递增）
//
// 注：「吞吐无显著回归>10%」判据已删除（MINOR 修复，2026-06-18）：
// 混合 flood 与 10k 纯写测试总量不同（600 vs 10000），直接对比无意义；
// 写后串行读也构造不出真并发场景。吞吐对照由 bench/stress 压测专题覆盖。
//
// ## 注意事项
//
// 读 Effect 走 `Effect::Persist{corr, ops:[StorageOp::Get]}` 路径，与写同队。
// 读的 PortReply 包含 `bytes`（scan 结果）或空 bytes（get miss），两者都是 Ok(PortOutcome)。
// QueryModule 只统计 PortReply 数，不关心具体 bytes。
//
// 迁移注记：本测试使用 NativeStorage/NativeEventSink/NativeHttp（native-specific），
// 因此放在 helix-driver-native/tests 而非 host/tests。
// 测试共享核（run_engine_loop from helix-driver-host），native 只提供具体 port 实现。

/// 读写混合 flood 模块：接受 "pump"（写）和 "query"（读）两种命令。
struct MixedModule {
    /// 写命令 corr 分配器（从 1 开始）
    next_write_corr: u64,
    /// 读命令 corr 分配器（从 100_001 开始，与写分离避免 corr 碰撞）
    next_read_corr: u64,
    /// 写 PortReply 计数
    write_replies: Arc<AtomicU64>,
    /// 读 PortReply 计数
    read_replies: Arc<AtomicU64>,
}

impl Module for MixedModule {
    fn name(&self) -> &'static str {
        "mixed-flood"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::Command(c) if c.name == "pump" || c.name == "query")
            || matches!(tick, Tick::PortReply { .. })
    }

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        use helix_core::effect::{GetSpec, SqlValue};
        match tick {
            Tick::Command(c) if c.name == "pump" => {
                let corr = helix_core::Correlation::from_raw(self.next_write_corr);
                self.next_write_corr += 1;
                out.push(Effect::Persist {
                    corr,
                    ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                        scope_key: "mixed:cursor".to_string(),
                        value: corr.raw() as i64,
                    })],
                });
            }
            Tick::Command(c) if c.name == "query" => {
                let corr = helix_core::Correlation::from_raw(self.next_read_corr);
                self.next_read_corr += 1;
                out.push(Effect::Persist {
                    corr,
                    ops: vec![StorageOp::Get(GetSpec {
                        table: "_helix_monotonic",
                        key_col: "scope_key",
                        key_val: SqlValue::Text("mixed:cursor".to_string()),
                    })],
                });
            }
            Tick::PortReply { corr, .. } => {
                if corr.raw() < 100_001 {
                    self.write_replies.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.read_replies.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_start(&mut self, _out: &mut EffectSink) -> Result<(), CoreError> {
        Ok(())
    }
}

/// P2 三入口混合 flood：500 写 + 100 读，无死锁，所有 PortReply 到达。
///
/// 回归锚: E1（泵死锁）· P2 engine 混合 flood（spec §5.2 场景 E）
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_engine_mixed_write_read_flood() {
    use helix_core::effect::{GetSpec, SqlValue};
    use helix_core::ports::Storage;

    const WRITES: u64 = 500;
    const READS: u64 = 100;
    const TOTAL: u64 = WRITES + READS;

    let db_path = "/tmp/helix_stress_engine_mixed.db";
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
    let _ = std::fs::remove_file(format!("{db_path}-wal"));

    let storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open mixed storage");
    let verify_storage = NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc"))
        .await
        .expect("open verify storage");
    let http = make_test_http();
    let (event_sink, _event_rx) = NativeEventSink::new();

    let write_replies = Arc::new(AtomicU64::new(0));
    let read_replies = Arc::new(AtomicU64::new(0));

    let mut shell = ExecutionShell::new();
    shell.register(MixedModule {
        next_write_corr: 1,
        next_read_corr: 100_001,
        write_replies: Arc::clone(&write_replies),
        read_replies: Arc::clone(&read_replies),
    });

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(512);
    let config = helix_driver_native::engine_loop::EngineConfig {
        storage,
        http,
        event_sink,
        max_http_inflight: 8,
    };
    let tick_tx_for_loop = tick_tx.clone();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config, shutdown_rx).await;
    });

    let start = Instant::now();

    // 发送 500 个写命令
    for _ in 0..WRITES {
        tick_tx
            .send(Tick::Command(AppCommand::new("pump", &b"{}"[..])))
            .await
            .expect("send pump command");
    }
    // 发送 100 个读命令（写命令发完后批量发读，dispatch 层并发调度混合负载）
    for _ in 0..READS {
        tick_tx
            .send(Tick::Command(AppCommand::new("query", &b"{}"[..])))
            .await
            .expect("send query command");
    }

    // 等所有 PortReply 到达（上限 120s）
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let wr = write_replies.load(Ordering::Relaxed);
        let rr = read_replies.load(Ordering::Relaxed);
        if wr >= WRITES && rr >= READS {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "P2 mixed flood timeout after 120s: writes={wr}/{WRITES}, reads={rr}/{READS} (possible deadlock)"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let elapsed = start.elapsed();
    let final_writes = write_replies.load(Ordering::Relaxed);
    let final_reads = read_replies.load(Ordering::Relaxed);

    // ① 无死锁（已在上面 deadline 断言）
    // ② 全部写 PortReply 到达
    assert_eq!(
        final_writes, WRITES,
        "写 PortReply 必须全部到达: {final_writes}/{WRITES}"
    );
    // ③ 全部读 PortReply 到达
    assert_eq!(
        final_reads, READS,
        "读 PortReply 必须全部到达: {final_reads}/{READS}"
    );

    // ④ cursor 最终值 ≥ WRITES（MAX guard）
    drop(tick_tx);
    // 短暂等待 engine 把 drain 收尾（不强制 graceful drain，只等短时间）
    tokio::time::sleep(Duration::from_millis(100)).await;

    let cursor_row = verify_storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("mixed:cursor".to_string()),
        })
        .await
        .expect("get cursor after mixed flood");

    let cursor_val = cursor_row
        .as_ref()
        .and_then(|r| r.iter().find(|(k, _)| k == "value"))
        .map(|(_, v)| v.clone());

    assert!(
        matches!(cursor_val, Some(SqlValue::Integer(v)) if v >= WRITES as i64),
        "cursor 终态应 ≥ {WRITES}，实际: {cursor_val:?}"
    );

    eprintln!("\n[stress_engine_mixed_write_read_flood] P2 mixed flood");
    eprintln!("  writes sent/replied  : {WRITES}/{final_writes}");
    eprintln!("  reads  sent/replied  : {READS}/{final_reads}");
    eprintln!(
        "  total time           : {:.3} ms",
        elapsed.as_secs_f64() * 1e3
    );
    eprintln!(
        "  mixed throughput     : {:.0} ops/s",
        TOTAL as f64 / elapsed.as_secs_f64()
    );
    eprintln!("  cursor final value   : {cursor_val:?}");
}
