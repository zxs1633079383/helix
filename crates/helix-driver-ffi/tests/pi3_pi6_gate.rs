//! PI-3 / PI-6 守门测试（HX-C011 可证伪：破坏机制即 fail）。
//!
//! - **PI-3**（HX-C007 批不变量）：一个 step 攒下的 N 条 Emit **恰好 1 次**过 FFI 边界。
//!   直接在 `FfiEventSink` 单元层证伪（不经全引擎，避免「单 step 产几条事件」随业务漂移）：
//!   `emit()` N 次（模拟一个 step 内 N 条 Emit）→ `flush()` 一次 → 断言回调**恰好 1 次**、且
//!   该次交付**全部 N 条**。破坏验证：若 flush 改成 emit 内逐条回调 → N 次回调、每次 1 条 → fail。
//!   与 f5_egress_parity（字节并集）正交：parity 测不到逐条破坏（断字节并集与回调**次数**无关）。
//!
//! - **PI-6**（命令背压）：channel 满返 `FFI_BUSY`，零静默丢弃、零无限缓冲。
//!   断言 `FFI_OK + FFI_BUSY == 发送总数`，且 cap 有界（必出现 BUSY）。破坏验证：try_send 改 send /
//!   cap 改 ∞ → BUSY 永不出现 → `busy > 0` fail。

use std::sync::Mutex as StdMutex;

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, HelixHandle, FFI_BUSY, FFI_OK,
};
use helix_driver_ffi::event_sink::FfiEventSink;
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::BatchSink;

extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

fn create_with(vtable: CppHostVtable) -> *mut HelixHandle {
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    // SAFETY: 指针/长度在调用期间有效；vtable 回调与 ctx 在 destroy 返回前有效。
    let handle = unsafe {
        helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
    };
    assert!(!handle.is_null(), "create 应成功");
    handle
}

// ── PI-3：FfiEventSink 单元层守门（每次回调的事件条数记录器）─────────────────
/// 记录每次 `on_domain_events_batch` 回调交付了多少条事件（per-invocation counts）。
type PerCallbackCounts = StdMutex<Vec<usize>>;

extern "C" fn counting_events(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    // SAFETY: ctx 指向 test 持有的 PerCallbackCounts（sink 存活期间有效）。
    let rec = unsafe { &*(ctx as *const PerCallbackCounts) };
    // SAFETY: ptr/len 由 FfiEventSink::flush 提供，回调期间有效（借出契约）。
    let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
    // 用 driver 的 decode 还原条数；非法/截断 → 记 0（不 panic，零信任）。
    let n = helix_driver_ffi::batch::decode_batch(buf)
        .map(|v| v.len())
        .unwrap_or(0);
    rec.lock().unwrap().push(n);
}

#[test]
fn pi3_n_emits_in_one_step_is_exactly_one_batch_callback() {
    let counts: Box<PerCallbackCounts> = Box::new(StdMutex::new(Vec::new()));
    let ctx = &*counts as *const PerCallbackCounts as *mut std::ffi::c_void;
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(counting_events),
        now_ms: Some(fixed_now_ms),
        ctx,
    };
    let sink = FfiEventSink::new(vtable);

    // 模拟一个 step 内攒 N 条 Emit（pump 每 step：dispatch 逐条 emit → step 末单次 flush）。
    let n = 5usize;
    for i in 0..n {
        sink.emit(DomainEventBytes(Bytes::from(format!(
            r#"{{"event":"e{i}","data":{{}}}}"#
        ))));
    }
    // step 末单次 flush：N 条应**恰好 1 次**过边界、整批交付。
    sink.flush();

    {
        let g = counts.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "PI-3 破坏：{n} 条 Emit 经 {} 次回调过边界——HX-C007 要求恰好 1 次（逐条则 ={n} 次）",
            g.len()
        );
        assert_eq!(g[0], n, "PI-3：单次批回调应交付全部 {n} 条，实际 {}", g[0]);
    }

    // 空 flush 不触发回调（防空批噪声）+ scratch 复用第二轮仍恰好 1 次（PI-4 协同）。
    sink.flush();
    sink.emit(DomainEventBytes(Bytes::from_static(
        br#"{"event":"e2","data":{}}"#,
    )));
    sink.flush();
    {
        let g = counts.lock().unwrap();
        assert_eq!(g.len(), 2, "空 flush 不回调；第二批仍单次");
        assert_eq!(g[1], 1, "第二批 1 条");
    }

    // ctx 是栈上 Box 借出（&*counts），sink drop 后不再回调，counts 随 test 栈帧释放。
    drop(sink);
}

// ── PI-6：背压（FFI_OK + FFI_BUSY == N，必出现 BUSY）─────────────────────────
extern "C" fn blocking_events(_ctx: *mut std::ffi::c_void, _ptr: *const u8, _len: usize) {
    // 阻塞 pump 线程足够久，让主线程把 tick channel（cap 256）灌满 → 暴露 FFI_BUSY。
    std::thread::sleep(std::time::Duration::from_millis(300));
}

#[test]
fn pi6_backpressure_ok_plus_busy_equals_total_no_silent_drop() {
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(blocking_events),
        now_ms: Some(fixed_now_ms),
        ctx: std::ptr::null_mut(),
    };
    let handle = create_with(vtable);

    // 发送远超 cap(256) 的命令；try_send 满 → FFI_BUSY。统计 OK / BUSY。
    let name = b"im_send_message";
    let payload = br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp","text":"x"}"#;
    let total = 2000usize;
    let mut ok = 0usize;
    let mut busy = 0usize;
    let mut other = 0usize;
    for _ in 0..total {
        // SAFETY: handle 存活；指针在调用期间有效。
        let code = unsafe {
            helix_command(
                handle,
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        match code {
            FFI_OK => ok += 1,
            FFI_BUSY => busy += 1,
            _ => other += 1, // FFI_NULL(已关闭) 等——不应在 destroy 前出现
        }
    }

    assert_eq!(
        other, 0,
        "destroy 前不应出现非 OK/BUSY 返回码（实际 other={other}）"
    );
    assert_eq!(
        ok + busy,
        total,
        "PI-6：FFI_OK({ok}) + FFI_BUSY({busy}) 必等于发送总数({total})——零静默丢弃"
    );
    // 破坏（cap 改 ∞ / try_send 改 send）→ 永不 BUSY → busy==0 fail。
    assert!(
        busy > 0,
        "PI-6 破坏：发 {total} 条命令（cap 256，回调阻塞中）竟无 FFI_BUSY——背压失效/无限缓冲"
    );

    // SAFETY: handle 由本 test 创建，destroy join pump（回调阻塞 sleep 结束后退出）。
    unsafe { helix_destroy(handle) };
}
