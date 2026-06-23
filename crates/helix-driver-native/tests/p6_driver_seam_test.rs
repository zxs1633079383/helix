//! P6 Tauri 四接缝集成测试（driver-native，module-agnostic）。
//!
//! 回归锚: P6 —— invoke(Tick::Command) → Module::handle → Effect::Emit → NativeEventSink →
//! `guard_bus_envelope`（接缝①出口守护）。验证 **engine 出口接线**（不耦合具体业务模块——
//! 用 FakeModule emit 等价 DomainEvent，对齐 engine_integration.rs 风格）：
//! 1. 真 engine 跑通命令 → emit DomainEvent（EventSink 计数）。
//! 2. `guard_bus_envelope`：channel `im:` 前缀且非 `im:__`（防自环）。
//! 3. 数组 data → `{items:[]}` 包装（M4，单一包装点在 driver 出口）。
//! 4. HX-C007 批处理：单 step 多 Effect 同批兑现（emit 全到达，不逐条丢）。
//!
//! bus_guard 纯函数细测在 `src/bus_guard.rs` `#[cfg(test)]`。

use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use helix_core::effect::{DomainEventBytes, Effect, EffectSink};
use helix_core::error::CoreError;
use helix_core::module_host::Module;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::HostNetworkConfig;

use helix_driver_native::{
    engine_loop::{run_engine_loop, EngineConfig},
    guard_bus_envelope, NativeEventSink, NativeHttp, NativeStorage,
};

/// 收命令后 emit 一组 DomainEvent 的测试模块（每个 emit_on_cmd 一条 Effect::Emit）。
struct EmitModule {
    /// 收到 Tick::Command 时要 emit 的事件载荷集（`{event,data}` JSON bytes）。
    emit_on_cmd: Vec<&'static [u8]>,
}

impl Module for EmitModule {
    fn name(&self) -> &'static str {
        "p6-seam-emit"
    }
    fn accepts(&self, tick: &Tick) -> bool {
        // Command 路由经 accepts（module_host：首个 accepts==true 处理）。
        matches!(tick, Tick::Command(_))
    }
    fn handle(&mut self, tick: &Tick, _now: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        if let Tick::Command(_) = tick {
            // 单 step 内一次性 push 全部 Emit（HX-C007 批处理：同 step Vec<Effect> 一批兑现）。
            for payload in &self.emit_on_cmd {
                out.push(Effect::Emit {
                    event: DomainEventBytes(Bytes::from_static(payload)),
                });
            }
        }
        Ok(())
    }
}

async fn make_temp_storage(path: &str) -> NativeStorage {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open temp storage")
}

fn make_test_http() -> NativeHttp {
    NativeHttp::with_config(HostNetworkConfig::new("http://127.0.0.1", "ws://127.0.0.1"))
        .expect("http")
}

const CHANNELS_LOADED: &[u8] =
    br#"{"event":"im:channels:loaded","data":{"reason":"delete_all_dialogs"}}"#;
const ARRAY_DATA_EVENT: &[u8] = br#"{"event":"im:channel:list","data":[{"id":"c1"},{"id":"c2"}]}"#;

async fn run_with(emit: Vec<&'static [u8]>, db: &str) -> (mpsc::Sender<Tick>, NativeEventSink) {
    let storage = make_temp_storage(db).await;
    let http = make_test_http();
    let (event_sink, _rx0) = NativeEventSink::new();
    let sink_clone = event_sink.clone();

    let mut shell = ExecutionShell::new();
    shell.register(EmitModule { emit_on_cmd: emit });

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
    (tick_tx, sink_clone)
}

fn cmd() -> Tick {
    Tick::Command(helix_core::tick::AppCommand::new(
        "im_delete_all_dialogs",
        Bytes::from_static(b"{}"),
    ))
}

/// 接缝②③①：invoke → emit im:channels:loaded → guard 判定 channel 合法（im: 非 im:__）。
#[tokio::test]
async fn invoke_emits_guarded_channels_loaded() {
    let (tick_tx, sink) = run_with(vec![CHANNELS_LOADED], "/tmp/helix_p6_seam_delete.db").await;
    let mut rx = sink.subscribe();
    tick_tx.send(cmd()).await.expect("send");

    let ev = tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("emit within 500ms")
        .expect("channel open");
    let g = guard_bus_envelope(&ev);
    assert_eq!(g.channel, "im:channels:loaded");
    assert!(g.channel_ok, "im: 前缀非 im:__ → 可 emit");
    assert_eq!(g.envelope["channel"], "im:channels:loaded");
    drop(tick_tx);
}

/// 接缝① M4：裸数组 data 事件经 engine emit → guard 出口包成 {items:[]}。
#[tokio::test]
async fn array_data_wrapped_in_items_at_emit_boundary() {
    let (tick_tx, sink) = run_with(vec![ARRAY_DATA_EVENT], "/tmp/helix_p6_seam_items.db").await;
    let mut rx = sink.subscribe();
    tick_tx.send(cmd()).await.expect("send");

    let ev = tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("emit")
        .expect("open");
    let g = guard_bus_envelope(&ev);
    assert!(g.channel_ok);
    assert_eq!(
        g.envelope["payload"]["data"],
        serde_json::json!({ "items": [ {"id": "c1"}, {"id": "c2"} ] }),
        "裸数组 data 必须在出口包成 {{items:[]}}"
    );
    drop(tick_tx);
}

/// HX-C007 批处理：单 step push 3 条 Emit → 全部到达 + 全 channel_ok（同批兑现，不逐条丢）。
#[tokio::test]
async fn batch_emits_all_arrive_and_pass_guard() {
    let (tick_tx, sink) = run_with(
        vec![CHANNELS_LOADED, ARRAY_DATA_EVENT, CHANNELS_LOADED],
        "/tmp/helix_p6_seam_batch.db",
    )
    .await;
    let mut rx = sink.subscribe();
    tick_tx.send(cmd()).await.expect("send");

    for _ in 0..3 {
        let ev = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("emit within 500ms")
            .expect("open");
        assert!(guard_bus_envelope(&ev).channel_ok);
    }
    drop(tick_tx);
}
