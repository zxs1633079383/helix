//! F5 — native↔FFI egress byte-level parity（跨端一致性显式证明）
//!
//! ## 目的（ROADMAP §一 M3 F5）
//!
//! 证明：对**同一份**已收集的 Emit 序列（source-of-truth），`NativeEventSink`（broadcast
//! channel）与 `FfiEventSink`（batch encode→vtable 回调→decode）两条 **egress 传输管道**
//! 交付的 DomainEventBytes byte-identical。
//!
//! 这是 egress 管道的「parity by construction」显式测试——两路走不同机制（broadcast 直传
//! vs batch encode/decode roundtrip），本测试把「两路对同一序列结果相同」从「设计信念」变为
//! 「可 CI 强制的事实」。**覆盖边界见末节「## 覆盖边界（诚实声明）」——勿当端到端引擎 parity。**
//!
//! ## 测试方法
//!
//! 1. 用公开 send/post 剧本及真实 SQLite Persist/PortReply 在 ExecutionShell
//!    下跑，收集所有 Effect::Emit 的 DomainEventBytes 作 source-of-truth 序列。
//! 2. **FfiEventSink 路径**：逐事件 emit → 按 step 边界 flush → 录 vtable 回调 buffer
//!    → batch.rs decode → Vec<DomainEventBytes>。
//! 3. **NativeEventSink 路径**：`NativeEventSink::new()` 裸 `broadcast::Receiver` → 逐事件 emit
//!    → `try_recv` → Vec<DomainEventBytes>（与 host-cli 现网 main.rs:170/243 裸 `recv()` 对齐）。
//! 4. 断言两 Vec byte-identical，且 == source-of-truth 序列。
//!
//! ## HX-C011 可证伪性
//!
//! 若 break FfiEventSink flush（如注释掉 flush 调用）→ FFI Vec 为空 → parity FAIL。
//! 若 break batch decode（如 off-by-one len）→ decode 内容错位 → parity FAIL。
//! commit body 附破坏证明（见提交记录）。
//!
//! ## 注意（runtime）
//!
//! 本测试不依赖 tokio runtime（broadcast `Receiver::recv()` 是 async，但我们 emit 后立即
//! `try_recv` 同步取；broadcast 的 send 同步，try_recv 直接取出无需 await）。
//!
//! ## 覆盖边界（诚实声明 · 勿过度解读）
//!
//! 1. **egress 管道 parity，非端到端引擎 parity**：两 sink 喂的是 `collect_source_emits`
//!    **单次**跑引擎预收集的同一份 source（`collect_ffi_emits(&source)` /
//!    `collect_native_emits(&source)`），只验证「batch roundtrip（HX-C007）vs broadcast 直传」
//!    对同一序列字节一致。引擎层双端一致靠**共享 `build_shell` by-construction**（同一份
//!    core/im 装配），未在本测试内被独立驱动证伪。本文件 A4 用例只证明“三份独立 shell
//!    注入相同 Tick 后业务输出一致”，**不证明 native/FFI 物理连接生产者会产同一 Tick 或
//!    保证相同先后**；真实 adapter 发布时序留给 A5 真链路验收。
//! 2. **§lagged（latent risk · 前移 M2）**：native `event_sink.rs` 有 lagged-aware
//!    `EventReceiver`/`subscribe_observed()`/`RecvOutcome::Lagged→resync_needed`（E7，
//!    helix-tauri event_bus 用）；`FfiEventSink`（`Mutex<Vec<Bytes>>` 整批 flush）**无对等
//!    lagged/resync 信号**。两路 byte-identical 仅在 **capacity 充裕、无慢消费**时成立；
//!    「lagged/丢事件下 native(resync_needed) vs FFI(batch) 是否对等」**未覆盖**——前移为
//!    M2 Tauri 接 `EventReceiver` 时验收，避免 native 升级后 parity 静默失效。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, StorageOp, TransportId};
use helix_core::ports::EventSink;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, ExecutionShell, Tick};
use helix_driver_ffi::batch::decode_batch;
use helix_driver_ffi::event_sink::FfiEventSink;
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, BatchSink, HostStorage};
use helix_driver_native::event_sink::NativeEventSink;
use helix_driver_web::{WebExecutor, WebOwnedEffect};
use helix_im::event::MessageV3Event;
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::Arc;

// ─── 真 Go post 历史抓帧（本测试仅本地回放）──────────────────────────────────────────────
const POST_JSON: &str = include_str!("../../helix-im/tests/fixtures/real_go_frames/post.json");

fn fixture_body(raw: &str) -> Vec<u8> {
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

const FIXED_NOW_MS: u64 = 1_700_000_000_000;

// ─── 引擎组装（共享 setup，两路分别喂同 DomainEvent 序列）──────────────────────

fn channel_id(n: u64) -> ChannelId {
    let s = format!("chfixx{n:020x}");
    ChannelId::from_str(&s).expect("test channel id fixture must be valid Id26")
}

fn build_shell() -> ExecutionShell {
    build_shell_with_mode(LocalStoreMode::Durable)
}

/// 用相同 Native 业务配置比较不同 egress 与本地存储模式的交付结果。
fn build_shell_with_mode(local_store_mode: LocalStoreMode) -> ExecutionShell {
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: "f5-viewer".to_string(),
        company_id: "f5-company".to_string(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
        client_platform: helix_im::ClientPlatform::Native,
        offline_sync_diagnostics: Default::default(),
    };
    let mut module = ImModule::new_with_local_store(config, local_store_mode);
    module.register_channel(channel_id(1), 0);
    let post_channel =
        ChannelId::from_str("rmibkwkjwidhdx5xy8pyse6a3r").expect("真帧 channelId 合法 26 字符");
    module.register_channel(post_channel, 40113);

    let mut shell = ExecutionShell::new();
    shell.register(module);
    shell
}

/// 兑现本轮 SQLite Effect 并按原 corr 回灌；不伪造 HTTP 成功或业务 Emit。
async fn drain_storage(
    shell: &mut ExecutionShell,
    storage: &HostStorage,
    initial: Vec<WebOwnedEffect>,
    now_ms: u64,
) -> Vec<DomainEventBytes> {
    let mut queue = VecDeque::from(initial);
    let mut events = Vec::new();
    let mut steps = 0;
    while let Some(effect) = queue.pop_front() {
        steps += 1;
        assert!(steps < 128, "storage replay must converge");
        let reply = match effect {
            WebOwnedEffect::Persist { corr, ops } => {
                Some((corr, execute_storage_ops(storage, &ops).await))
            }
            WebOwnedEffect::PersistAtomic { corr, ops } => {
                Some((corr, execute_storage_ops_atomic(storage, &ops).await))
            }
            WebOwnedEffect::PersistFire { ops } => {
                assert!(matches!(
                    execute_storage_ops(storage, &ops).await,
                    PortOutcome::Ok(_)
                ));
                None
            }
            WebOwnedEffect::Emit { event } => {
                events.push(event);
                None
            }
            _ => None,
        };
        if let Some((corr, outcome)) = reply {
            assert!(
                matches!(&outcome, PortOutcome::Ok(_)),
                "SQLite reply: {outcome:?}"
            );
            let next = shell
                .step(Tick::PortReply { corr, outcome }, now_ms + steps)
                .expect("storage PortReply");
            queue.extend(next.iter().map(WebOwnedEffect::from));
        }
    }
    events
}

/// 用真实 SQLite 回灌启动、乐观写、post 原子提交与成员读回，收集生产 Emit。
async fn collect_source_emits(shell: &mut ExecutionShell) -> Vec<DomainEventBytes> {
    let storage = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .expect("isolated SQLite");
    for sql in helix_im::IM_SCHEMA
        .iter()
        .chain(helix_im::IM_SCHEMA_MIGRATIONS)
    {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "schema: {error:?}"
            );
        }
    }
    let start = shell
        .start()
        .expect("start")
        .iter()
        .map(WebOwnedEffect::from)
        .collect();
    let mut out = drain_storage(shell, &storage, start, FIXED_NOW_MS).await;
    let payload = serde_json::json!({
        "channel_id": "rmibkwkjwidhdx5xy8pyse6a3r",
        "allocated_temporary_id": "helixverify1781671289",
        "text": "hello-f5"
    });
    let cmd = Tick::Command(AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&payload).unwrap(),
    ));
    let effects = shell.step(cmd, FIXED_NOW_MS + 200).expect("send command");
    assert!(
        !contains_event(effects, "im:post:sending"),
        "sending requires PersistOk"
    );
    let pending = effects.iter().map(WebOwnedEffect::from).collect();
    out.extend(drain_storage(shell, &storage, pending, FIXED_NOW_MS + 200).await);
    let post_tick = Tick::Inbound(InboundBytes(fixture_body(POST_JSON).into()));
    let effects = shell
        .step(post_tick, FIXED_NOW_MS + 400)
        .expect("post inbound");
    assert!(
        !contains_event(effects, "im:post:received"),
        "received requires atomic commit/readback"
    );
    let pending = effects.iter().map(WebOwnedEffect::from).collect();
    out.extend(drain_storage(shell, &storage, pending, FIXED_NOW_MS + 400).await);
    for expected in ["im:post:sending", "im:post:received"] {
        assert_eq!(
            out.iter()
                .filter(|event| {
                    serde_json::from_slice::<serde_json::Value>(&event.0).unwrap()["event"]
                        == expected
                })
                .count(),
            1,
            "exactly one {expected} must be produced"
        );
    }
    shell.stop().expect("stop replay shell");
    out
}

// ─── FfiEventSink 路径 ───────────────────────────────────────────────────────

/// vtable 回调：把 batch buffer 追加到 `batches` Vec（测试侧录制）。
struct BatchCapture {
    batches: Arc<Mutex<Vec<Vec<u8>>>>,
}

/// 构造 CppHostVtable + 共享 capture buffer。
///
/// 使用 Box::into_raw 把 capture 传给 vtable ctx（单 owner，FFI 铁律 HX-C006）。
fn make_test_vtable() -> (CppHostVtable, Arc<Mutex<Vec<Vec<u8>>>>) {
    let batches = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let batches_clone = Arc::clone(&batches);

    // SAFETY: BatchCapture 由 Box::into_raw 转出给 vtable.ctx；仅在 on_domain_events_batch
    // 回调期间经 ctx 取回（Box::from_raw），不逃逸回调，单 owner。
    let capture_box = Box::new(BatchCapture {
        batches: batches_clone,
    });
    let ctx_ptr = Box::into_raw(capture_box) as *mut c_void;

    // SAFETY: on_domain_events_batch 接 ctx（*mut BatchCapture）+ ptr/len（batch buffer），
    // 在回调期间把 bytes 拷贝进 batches Vec，不持有 ptr 逃逸（buf 由调用方栈帧持有到回调返回）。
    unsafe extern "C" fn on_batch(ctx: *mut c_void, ptr: *const u8, len: usize) {
        // SAFETY: ctx 由 make_test_vtable 的 Box::into_raw 创建，在本回调期间有效（vtable 生命周期内）。
        let capture = &*(ctx as *mut BatchCapture);
        // SAFETY: ptr..ptr+len 由 FfiEventSink::flush 的 buf 在其栈帧持有，回调期间有效。
        let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
        capture.batches.lock().push(bytes);
    }

    let vtable = CppHostVtable {
        on_domain_events_batch: Some(on_batch),
        now_ms: None, // 测试不通过 vtable 取时间
        ctx: ctx_ptr,
    };

    (vtable, batches)
}

/// 经 FfiEventSink 路径收集 DomainEventBytes（emit → flush per step → decode）。
fn collect_ffi_emits(source_emits: &[DomainEventBytes]) -> Vec<DomainEventBytes> {
    let (vtable, batches_arc) = make_test_vtable();
    let sink = FfiEventSink::new(vtable);

    // 模拟 pump 每 step 边界 flush 语义：
    // source_emits 已是线性序列（非按 step 分组），测试简化为
    // 逐条 emit + 单次 flush（等价于单步多 emit + step 末 flush，匹配 pump 语义）。
    for ev in source_emits {
        sink.emit(ev.clone());
    }
    sink.flush();

    // 从录制的 batches 中 decode 所有条目
    let batches = batches_arc.lock();
    assert_eq!(
        batches.len(),
        1,
        "one non-empty flush must invoke one batch callback"
    );
    let mut out = Vec::new();
    for batch_buf in batches.iter() {
        let decoded = decode_batch(batch_buf)
            .expect("FfiEventSink 产出的 batch buffer 必须能 decode（encode/decode roundtrip）");
        for item in decoded {
            out.push(DomainEventBytes(Bytes::copy_from_slice(item)));
        }
    }

    // 清理 vtable ctx（归还 Box，防 memory leak）
    drop(batches);
    // SAFETY: vtable.ctx 由 make_test_vtable 的 Box::into_raw 创建，现在归还所有权，单 owner。
    unsafe {
        let _ = Box::from_raw(vtable.ctx as *mut BatchCapture);
    }

    out
}

// ─── NativeEventSink 路径 ────────────────────────────────────────────────────

/// 经 NativeEventSink 路径收集 DomainEventBytes（emit → try_recv 同步取）。
fn collect_native_emits(source_emits: &[DomainEventBytes]) -> Vec<DomainEventBytes> {
    let (sink, mut rx) = NativeEventSink::new();
    for ev in source_emits {
        sink.emit(ev.clone());
    }

    let mut out = Vec::new();
    // try_recv 同步取：所有 emit 在 broadcast 中是同步的，无需 await。
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

// ─── 测试 ────────────────────────────────────────────────────────────────────

/// 主测试：native↔FFI egress byte-level parity。
///
/// HX-C011 可证伪：若 break FfiEventSink flush 或 batch decode → parity FAIL。
#[tokio::test]
async fn f5_egress_parity_native_ffi_byte_identical() {
    // 跑剧本，收集 source-of-truth Emit 序列
    let mut shell = build_shell();
    let source = collect_source_emits(&mut shell).await;
    assert!(
        !source.is_empty(),
        "剧本必须产出至少一条 Emit（否则 parity 测试无意义）"
    );

    // FfiEventSink 路径
    let ffi_vec = collect_ffi_emits(&source);

    // NativeEventSink 路径
    let native_vec = collect_native_emits(&source);

    // 长度一致
    assert_eq!(
        ffi_vec.len(),
        native_vec.len(),
        "FfiEventSink 交付{}条，NativeEventSink 交付{}条——条数不一致",
        ffi_vec.len(),
        native_vec.len()
    );
    assert_eq!(
        ffi_vec.len(),
        source.len(),
        "两路交付条数({}) 必须 == source-of-truth({})，防 drop 静默",
        ffi_vec.len(),
        source.len()
    );

    // byte-identical 逐条断言
    for (i, (ffi, native)) in ffi_vec.iter().zip(native_vec.iter()).enumerate() {
        assert_eq!(
            ffi.0,
            native.0,
            "DomainEvent[{i}] FFI 与 native 字节不同:\n  ffi    = {}\n  native = {}",
            String::from_utf8_lossy(&ffi.0),
            String::from_utf8_lossy(&native.0),
        );
    }

    // source-of-truth 校验（每路都 == 原始 Emit）
    for (i, (actual, expected)) in ffi_vec.iter().zip(source.iter()).enumerate() {
        assert_eq!(
            actual.0,
            expected.0,
            "DomainEvent[{i}] FFI 与 source 字节不同:\n  ffi    = {}\n  source = {}",
            String::from_utf8_lossy(&actual.0),
            String::from_utf8_lossy(&expected.0),
        );
    }
}

/// 证明 MessageV3 业务信封经过 FFI 批次后保持逐字节不变。
#[test]
fn ffi_callback_delivers_message_v3_event_without_ui_wrapper() {
    let event = match MessageV3Event::new(
        "im:post:received",
        serde_json::json!({
            "id": "post-1",
            "channelId": "channel-1",
            "userId": "user-1",
            "message": "hello",
            "createAt": 1,
            "sendStatus": "sent"
        }),
    ) {
        Ok(event) => event,
        Err(error) => panic!("MessageV3 fixture must encode: {error}"),
    };
    let Effect::Emit { event } = event.into_effect() else {
        panic!("MessageV3 event must lower to Emit");
    };

    let delivered = collect_ffi_emits(std::slice::from_ref(&event));
    assert_eq!(
        delivered.len(),
        1,
        "one MessageV3 event must cross one FFI batch"
    );
    assert_eq!(delivered[0].0, event.0, "FFI must not add a UI wrapper");
    let envelope: serde_json::Value = match serde_json::from_slice(delivered[0].0.as_ref()) {
        Ok(value) => value,
        Err(error) => panic!("callback event must stay valid JSON: {error}"),
    };
    assert_eq!(envelope["event"], "im:post:received");
    assert!(envelope.get("schemaVersion").is_none());
    assert!(envelope.get("mutations").is_none());
}

/// 辅助验证：FfiEventSink 空缓冲 flush 不触发回调（防空批噪声）。
#[test]
fn f5_ffi_sink_empty_flush_no_callback() {
    let (vtable, batches_arc) = make_test_vtable();
    let sink = FfiEventSink::new(vtable);
    // 不 emit 任何事件，直接 flush
    sink.flush();
    assert!(
        batches_arc.lock().is_empty(),
        "空缓冲 flush 不应触发 on_domain_events_batch 回调"
    );
    // 清理 ctx
    // SAFETY: 同 make_test_vtable 契约，单 owner 归还。
    unsafe {
        let _ = Box::from_raw(vtable.ctx as *mut BatchCapture);
    }
}

#[derive(Clone, Copy)]
enum LifecycleStep {
    Connected,
    Hello(&'static str),
    Disconnected,
}

fn lifecycle_tick(step: LifecycleStep) -> Tick {
    match step {
        LifecycleStep::Connected => Tick::Connected(TransportId::from_raw(0)),
        LifecycleStep::Hello(connection_id) => Tick::Inbound(InboundBytes(
            serde_json::to_vec(&serde_json::json!({
                "action": "hello",
                "data": { "connectionId": connection_id }
            }))
            .expect("static hello JSON")
            .into(),
        )),
        LifecycleStep::Disconnected => Tick::Disconnected(TransportId::from_raw(0)),
    }
}

fn effect_fingerprints(effects: &[Effect]) -> Vec<String> {
    effects.iter().map(|effect| format!("{effect:?}")).collect()
}

fn web_effect_fingerprints(effects: &[WebOwnedEffect]) -> Vec<String> {
    effects.iter().map(|effect| format!("{effect:?}")).collect()
}

fn contains_event(effects: &[Effect], expected: &str) -> bool {
    effects.iter().any(|effect| match effect {
        Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(&event.0)
            .ok()
            .and_then(|value| {
                value
                    .get("event")
                    .and_then(|name| name.as_str())
                    .map(str::to_owned)
            })
            .is_some_and(|name| name == expected),
        _ => false,
    })
}

/// A4：native / FFI 两份独立 shell 与 Web owned executor 注入相同生命周期 Tick，
/// 逐 step 业务 Effect 必须一致。
///
/// 这是共享业务核的确定性契约，不是三端物理连接 adapter parity 测试。
#[test]
fn a4_identical_injected_lifecycle_script_is_three_platform_equivalent() {
    let mut native_shell = build_shell();
    let mut ffi_shell = build_shell();
    let mut web_executor = WebExecutor::new(build_shell_with_mode(LocalStoreMode::Session));
    let native_start = effect_fingerprints(native_shell.start().expect("native start"));
    assert_eq!(
        native_start,
        effect_fingerprints(ffi_shell.start().expect("ffi start")),
        "native/FFI 独立 shell 的启动 Effect 必须一致"
    );
    assert_eq!(
        native_start,
        web_effect_fingerprints(&web_executor.start().expect("Web start")),
        "Web executor 的启动 Effect 必须与 native/FFI 一致"
    );

    let script = [
        (LifecycleStep::Connected, FIXED_NOW_MS),
        (LifecycleStep::Hello("A4_CONN_1"), FIXED_NOW_MS + 1),
        (LifecycleStep::Disconnected, FIXED_NOW_MS + 2),
        (LifecycleStep::Connected, FIXED_NOW_MS + 3),
        (LifecycleStep::Hello("A4_CONN_2"), FIXED_NOW_MS + 4),
    ];

    for (index, (step, now_ms)) in script.into_iter().enumerate() {
        let native = native_shell
            .step(lifecycle_tick(step), now_ms)
            .expect("native lifecycle step");
        let native_fingerprint = effect_fingerprints(native);
        let watermark_corr = native.iter().find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        StorageOp::ScopedMax(spec)
                            if spec.table == "message" && spec.value_col == "create_at"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        });
        if matches!(step, LifecycleStep::Hello(_)) {
            assert!(
                !native
                    .iter()
                    .any(|effect| matches!(effect, Effect::Http { .. })),
                "hello must read the local watermark before increment HTTP"
            );
            assert!(
                contains_event(native, "im:connection:established"),
                "hello step 必须有连接建立事件，防两边都空的假绿"
            );
        }

        let ffi = ffi_shell
            .step(lifecycle_tick(step), now_ms)
            .expect("ffi lifecycle step");
        assert_eq!(
            native_fingerprint,
            effect_fingerprints(ffi),
            "生命周期剧本第 {index} 步 native/FFI 业务 Effect 漂移"
        );
        let web = web_executor
            .step(lifecycle_tick(step), now_ms)
            .expect("Web lifecycle step");
        assert_eq!(
            native_fingerprint,
            web_effect_fingerprints(&web),
            "生命周期剧本第 {index} 步 Web 业务 Effect 漂移"
        );
        if matches!(step, LifecycleStep::Hello(_)) {
            let corr = watermark_corr.expect("hello must request a message watermark scan");
            let reply = || Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[{\"create_at\":1234}]"))),
            };
            let native = native_shell
                .step(reply(), now_ms)
                .expect("native watermark reply");
            let requests: Vec<_> = native
                .iter()
                .filter_map(|effect| match effect {
                    Effect::Http { req, .. } if req.url.ends_with("/channels/load/increment") => {
                        Some(req)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                requests.len(),
                1,
                "one increment follows the matching scan reply"
            );
            let body: serde_json::Value =
                serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
            assert_eq!(
                body["timestamp"], 1234,
                "increment uses persisted watermark"
            );
            let expected = effect_fingerprints(native);
            assert_eq!(
                expected,
                effect_fingerprints(ffi_shell.step(reply(), now_ms).unwrap())
            );
            assert_eq!(
                expected,
                web_effect_fingerprints(&web_executor.step(reply(), now_ms).unwrap())
            );
        }
    }
}
