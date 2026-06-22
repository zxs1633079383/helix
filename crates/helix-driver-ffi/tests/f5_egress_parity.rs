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
//! 1. 用 F5 golden 剧本（与 helix-im/tests/f5_golden_replay.rs 同参数）在 ExecutionShell
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
//!    core/im 装配），未在本测试内被独立驱动证伪。回归「引擎双端 parity」需另立测试
//!    （两端各自 `shell.step` 同剧本再比对）。
//! 2. **§lagged（latent risk · 前移 M2）**：native `event_sink.rs` 有 lagged-aware
//!    `EventReceiver`/`subscribe_observed()`/`RecvOutcome::Lagged→resync_needed`（E7，
//!    helix-tauri event_bus 用）；`FfiEventSink`（`Mutex<Vec<Bytes>>` 整批 flush）**无对等
//!    lagged/resync 信号**。两路 byte-identical 仅在 **capacity 充裕、无慢消费**时成立；
//!    「lagged/丢事件下 native(resync_needed) vs FFI(batch) 是否对等」**未覆盖**——前移为
//!    M2 Tauri 接 `EventReceiver` 时验收，避免 native 升级后 parity 静默失效。

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_core::tick::{AppCommand, InboundBytes};
use helix_core::{Effect, ExecutionShell, Tick};
use helix_driver_ffi::batch::decode_batch;
use helix_driver_ffi::event_sink::FfiEventSink;
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::BatchSink;
use helix_driver_native::event_sink::NativeEventSink;
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use parking_lot::Mutex;
use std::ffi::c_void;
use std::sync::Arc;

// ─── 真 Go post 帧（同 golden 测试）──────────────────────────────────────────────
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
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: String::new(),
        company_id: String::new(),
    };
    let mut module = ImModule::new(config);
    module.register_channel(channel_id(1), 0);
    let post_channel =
        ChannelId::from_str("rmibkwkjwidhdx5xy8pyse6a3r").expect("真帧 channelId 合法 26 字符");
    module.register_channel(post_channel, 40113);

    let mut shell = ExecutionShell::new();
    shell.register(module);
    shell
}

/// 跑剧本，收集原始 Emit 序列（source-of-truth）。
fn collect_source_emits(shell: &mut ExecutionShell) -> Vec<DomainEventBytes> {
    let mut out = Vec::new();

    let start_effects = shell.start().expect("start should not fail");
    collect_emits_from_slice(start_effects, &mut out);

    let cmd_payload = serde_json::json!({
        "channel_id": "00000000000000000000000001",
        "temporary_id": "f5_tmp_1",
        "text": "hello-f5"
    });
    let cmd = Tick::Command(AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&cmd_payload).expect("static JSON"),
    ));
    let effects = shell.step(cmd, FIXED_NOW_MS).expect("step cmd");
    collect_emits_from_slice(effects, &mut out);

    let post_tick = Tick::Inbound(InboundBytes(fixture_body(POST_JSON).into()));
    let effects = shell.step(post_tick, FIXED_NOW_MS).expect("step inbound");
    collect_emits_from_slice(effects, &mut out);

    out
}

fn collect_emits_from_slice(effects: &[Effect], out: &mut Vec<DomainEventBytes>) {
    for e in effects {
        if let Effect::Emit { event } = e {
            out.push(event.clone());
        }
    }
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
#[test]
fn f5_egress_parity_native_ffi_byte_identical() {
    // 跑剧本，收集 source-of-truth Emit 序列
    let mut shell = build_shell();
    let source = collect_source_emits(&mut shell);
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
