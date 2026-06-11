//! 集成自测：engine_loop 端到端闭环
//!
//! 验证：
//! 1. Timer 到期产出 Tick::Timer（进入 engine handle）
//! 2. Emit 可被订阅端收到
//! 3. Persist → PortReply 回灌闭环（内存 SQLite）
//!
//! 使用假模块（FakeModule）驱动 ExecutionShell，不依赖 helix-im。
//!
//! ## 数据库策略
//!
//! 使用 `sqlite::memory:` 单连接（所有三池共用同一 URL，SQLite in-memory 在
//! 每个连接独立，但 max=1 的池等价于单连接）。仅 sync_write_pool 建表、写入；
//! 读路径测试通过 PortReply 的 Err/Ok 状态判断（不验证读出内容）。

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;

use helix_core::{ExecutionShell, Tick};
use helix_core::effect::{Correlation, Effect, EffectSink, TimerId,
                          DomainEventBytes, StorageOp};
use helix_core::error::CoreError;
use helix_core::module_host::Module;

use helix_driver_native::{
    NativeStorage, NativeHttp, NativeEventSink,
    engine_loop::{EngineConfig, run_engine_loop},
};

// ─── 假模块（测试专用）────────────────────────────────────────────────────────

/// 记录所有收到的 Tick 名称，按指令产出 Effect 的测试模块。
struct FakeModule {
    name: &'static str,
    start_effects: Vec<BuiltEffect>,
    /// 接收到的 Tick 记录（与测试代码共享）
    received: Arc<std::sync::Mutex<Vec<String>>>,
}

/// 预先构建好的 Effect（避免 on_start / handle 里的借用问题）
enum BuiltEffect {
    PersistUpsert { corr_raw: u64 },
    ScheduleTimer { id_raw: u64, after_ms: u64 },
    Emit { payload: &'static [u8] },
}

impl FakeModule {
    fn new(
        name: &'static str,
        start_effects: Vec<BuiltEffect>,
        received: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        Self { name, start_effects, received }
    }
}

impl Module for FakeModule {
    fn name(&self) -> &'static str { self.name }

    fn accepts(&self, _: &Tick) -> bool { false }

    fn handle(
        &mut self,
        tick: &Tick,
        _now_ms: u64,
        _out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        let label = match tick {
            Tick::PortReply { corr, outcome } => format!(
                "PortReply({},{})",
                corr.raw(),
                if matches!(outcome, helix_core::tick::PortOutcome::Ok(_)) { "ok" } else { "err" }
            ),
            Tick::Timer(id) => format!("Timer({})", id.raw()),
            Tick::Inbound(_) => "Inbound".to_string(),
            Tick::Command(c) => format!("Command({})", c.name),
            Tick::Connected(_) => "Connected".to_string(),
            Tick::Disconnected(_) => "Disconnected".to_string(),
        };
        if let Ok(mut g) = self.received.lock() {
            g.push(label);
        }
        Ok(())
    }

    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        for e in &self.start_effects {
            match e {
                BuiltEffect::PersistUpsert { corr_raw } => {
                    // monotonic_upsert（不需要预建表）
                    out.push(Effect::Persist {
                        corr: Correlation::from_raw(*corr_raw),
                        ops: vec![StorageOp::MonotonicUpsert(
                            helix_core::effect::MonotonicUpsertSpec {
                                scope_key: format!("test_key_{}", corr_raw),
                                value: 1,
                            }
                        )],
                    });
                }
                BuiltEffect::ScheduleTimer { id_raw, after_ms } => {
                    out.push(Effect::ScheduleTimer {
                        id: TimerId::from_raw(*id_raw),
                        after_ms: *after_ms,
                    });
                }
                BuiltEffect::Emit { payload } => {
                    out.push(Effect::Emit {
                        event: DomainEventBytes(Bytes::from_static(payload)),
                    });
                }
            }
        }
        Ok(())
    }
}

// ─── 辅助：创建测试用 NativeStorage（文件临时 DB）─────────────────────────────

async fn make_temp_storage(path: &str) -> NativeStorage {
    // 删除旧文件（忽略错误）
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open temp storage")
}

// ─── 测试 1：Emit 可被订阅端收到 ─────────────────────────────────────────────
//
// 最简单的测试：on_start 产出 Emit，订阅者接收。

#[tokio::test]
async fn test_emit_received_by_subscriber() {
    let storage = make_temp_storage("/tmp/helix_test_emit.db").await;
    let http = NativeHttp::new().expect("http");
    let (event_sink, mut event_rx) = NativeEventSink::new();

    let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let payload: &'static [u8] = b"test-domain-event-emit";

    let mut shell = ExecutionShell::new();
    shell.register(FakeModule::new(
        "emit-test",
        vec![BuiltEffect::Emit { payload }],
        Arc::clone(&received),
    ));

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let config = EngineConfig { storage, http, event_sink };
    let tick_tx_for_loop = tick_tx.clone();

    let _loop_handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config).await;
    });

    // 等待 emit 事件（最多 500ms）
    let event = tokio::time::timeout(
        Duration::from_millis(500),
        event_rx.recv(),
    )
    .await
    .expect("should receive event within 500ms")
    .expect("channel should not be closed");

    assert_eq!(event.0, Bytes::from_static(payload));
    drop(tick_tx);
}

// ─── 测试 2：Timer 到期产出 Tick::Timer，engine handle 被调用 ─────────────────

#[tokio::test]
async fn test_timer_fires_into_engine() {
    let storage = make_temp_storage("/tmp/helix_test_timer.db").await;
    let http = NativeHttp::new().expect("http");
    let (event_sink, _) = NativeEventSink::new();

    let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let received_clone = Arc::clone(&received);

    let mut shell = ExecutionShell::new();
    shell.register(FakeModule::new(
        "timer-test",
        // on_start: 调度 50ms 后触发的 timer
        vec![BuiltEffect::ScheduleTimer { id_raw: 77, after_ms: 50 }],
        received_clone,
    ));

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let config = EngineConfig { storage, http, event_sink };
    let tick_tx_for_loop = tick_tx.clone();

    let _loop_handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config).await;
    });

    // 等待 timer 触发（50ms + 500ms margin）
    tokio::time::sleep(Duration::from_millis(300)).await;

    let ticks = received.lock().unwrap();
    assert!(
        ticks.iter().any(|s| s.contains("Timer(77)")),
        "expected Timer(77) in received ticks, got: {:?}", &*ticks
    );
    drop(tick_tx);
}

// ─── 测试 3：Persist (monotonic_upsert) → PortReply 回灌闭环 ─────────────────
//
// monotonic_upsert 自建 _helix_monotonic 表，无需预建表。

#[tokio::test]
async fn test_persist_portreply_roundtrip() {
    let storage = make_temp_storage("/tmp/helix_test_persist.db").await;
    let http = NativeHttp::new().expect("http");
    let (event_sink, _) = NativeEventSink::new();

    let received = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let received_clone = Arc::clone(&received);

    let corr_raw = 2002u64;

    let mut shell = ExecutionShell::new();
    shell.register(FakeModule::new(
        "persist-test",
        vec![BuiltEffect::PersistUpsert { corr_raw }],
        received_clone,
    ));

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let config = EngineConfig { storage, http, event_sink };
    let tick_tx_for_loop = tick_tx.clone();

    let _loop_handle = tokio::spawn(async move {
        run_engine_loop(shell, tick_rx, tick_tx_for_loop, config).await;
    });

    // 等待 Persist 完成并 PortReply 回灌到 engine，handle 被调用
    tokio::time::sleep(Duration::from_millis(500)).await;

    let ticks = received.lock().unwrap();
    assert!(
        ticks.iter().any(|s| s.starts_with("PortReply(2002,")),
        "expected PortReply(2002,...) in received ticks, got: {:?}", &*ticks
    );
    drop(tick_tx);
}
