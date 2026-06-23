//! 生命周期 ↔ 网络 I/O 解耦回归（fix/lifecycle-net-decouple）
//!
//! ## 根因（systematic-debugging 已确证 · instrumentation 实测）
//!
//! 连不上服务器（blackhole IP 不可达）时，`helix_create`/`helix_destroy` 各卡 30s
//! （`DEFAULT_TIMEOUT_SECS=30`，合计 60s）。两个**独立**的同步网络 await：
//! - ① **create**：`pump.rs` 装配阶段 `ws.connect().await` 同步等 WS 连接（连不上等满 30s）。
//! - ② **destroy**：graceful drain `http_pool.shutdown().await`（必达 Http 池）无超时 join，
//!   `im_send_message` 产的必达 `Http`(POST /posts/create) reqwest 卡满 30s timeout。
//!
//! ## 根治（非加 connect_timeout 治标——用户明确否决治标）
//!
//! - ① create connect 异步化：装配阶段**不 await** `ws.connect()`，spawn 后台连接任务，
//!   `ready_tx` 立即回传 → create 不等网络。
//! - ② destroy 必达 Http drain 给合理上限（`tokio::time::timeout`），超时放弃在途网络
//!   （乐观态 status=Sending 已落库，是真源；不丢保证靠重连重发对账兜底）。
//!
//! ## HX-C011 可证伪性（上界断言 · 禁下界）
//!
//! 本测试断言「连不上时 create→command→destroy 总耗时 **< 2s**」（上界）。当前 60s → FAIL（红）。
//! 还原任一阻塞 await（create 同步 connect / destroy 无超时 drain）→ 总耗时回到 ~30-60s → FAIL。
//! 仅根治两处后才 PASS（绿）。配结构化证（见 `create_returns_before_ws_connected`）。

use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use helix_driver_ffi::api::{helix_command, helix_create_with_network, helix_destroy, FFI_OK};
use helix_driver_ffi::vtable::CppHostVtable;

extern "C" fn noop_events(_ctx: *mut std::ffi::c_void, _ptr: *const u8, _len: usize) {}
extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

fn test_vtable() -> CppHostVtable {
    CppHostVtable {
        on_domain_events_batch: Some(noop_events),
        now_ms: Some(fixed_now_ms),
        ctx: std::ptr::null_mut(),
    }
}

/// 不可达 host = **blackhole IP**（`192.0.2.1` = TEST-NET-1，RFC 5737「保留作文档」，全网不可路由）。
///
/// ⚠️ 为什么不用 `*.example.test`：本机 DNS（及很多企业 / captive 解析器）把未知 TLD 重定向到
/// `198.18.0.0/15` benchmarking 段的占位 IP，连接**立即 refuse**（< 50ms），**复现不出 30s hang**。
/// 30s 的根因是 TCP SYN 被**黑洞丢弃**（无 RST 回包）→ reqwest/tungstenite 等满 `DEFAULT_TIMEOUT_SECS`。
/// `192.0.2.1` 保证 SYN 被丢弃（不可路由地址），是 hermetic（无外网依赖）且**稳定 hang** 的复现源。
/// 端口 9（discard）进一步确保即便可路由也无服务监听。
const UNREACHABLE_API: &[u8] = b"https://192.0.2.1:9";
const UNREACHABLE_WS: &[u8] = b"ws://192.0.2.1:9/socket";

/// 上界回归：连不上 host 时，create → 发一条必达 Http 命令 → destroy 总耗时 < 2s。
///
/// 修复前：create 30s（同步 connect）+ destroy 30s（无超时 drain 等必达 Http reqwest）= ~60s。
/// 修复后：create 立即返回（connect 后台化）+ destroy ≤ drain 上限（~3s）→ 总 < 2s（命令本身瞬时入队）。
///
/// 注意：本测试**默认会等 30-60s**直到修复落地——这正是「先失败测试」的红状态。
#[test]
fn create_command_destroy_fast_when_unreachable() {
    let t0 = Instant::now();

    // SAFETY: 测试栈上构造的有效 UTF-8 指针/长度，vtable 函数指针指向本模块静态 fn。
    let handle = unsafe {
        helix_create_with_network(
            UNREACHABLE_API.as_ptr(),
            UNREACHABLE_API.len(),
            UNREACHABLE_WS.as_ptr(),
            UNREACHABLE_WS.len(),
            test_vtable(),
        )
    };
    assert!(
        !handle.is_null(),
        "create 即使连不上也应成功返回（连不上仍进泵——pump.rs:109 意图）"
    );

    // 发一条必达 Http 命令（im_send_message → Effect::Http POST /posts/create，连不上 reqwest 卡 30s）。
    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-net-decouple","text":"hi"}"#;
    // SAFETY: 同上，handle 来自 create，name/payload 指向测试栈上有效内存。
    let status = unsafe {
        helix_command(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
        )
    };
    assert_eq!(status, FFI_OK, "命令入队应成功（非阻塞 try_send）");

    // 销毁：graceful drain 不应无限等在途必达 Http（修复后给 drain 上限）。
    // SAFETY: handle 来自 create，单次 destroy（独占所有权契约）。
    unsafe { helix_destroy(handle) };

    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "连不上 host 时 create→command→destroy 总耗时应 < 2s（生命周期不被网络阻塞），实际 {:?}",
        elapsed
    );
}

/// 结构化证（非时序）：create 返回时 WS **尚未连上**——证明 connect 已异步化（不在 create 同步路径）。
///
/// HX-C011：这是结构化不变量断言（「连不上的 host，create 返回即说明它没在装配阶段同步等连接」），
/// 不依赖墙钟下界。配合上面的上界耗时断言，双向锚定「connect 是 spawn 的、不在 create 同步路径」。
///
/// 我们无法从 FFI opaque handle 直接读 WS 态，故用**间接结构化证**：create 在不可达 host 下
/// 仍**秒级**返回（< 2s），等价于「create 同步路径未含 30s connect await」。若 connect 仍同步，
/// 此断言必因 create 自身耗时 ~30s 而 FAIL。
#[test]
fn create_returns_before_ws_connected() {
    let t0 = Instant::now();
    // SAFETY: 测试栈上有效指针/长度 + 静态 fn 指针 vtable。
    let handle = unsafe {
        helix_create_with_network(
            UNREACHABLE_API.as_ptr(),
            UNREACHABLE_API.len(),
            UNREACHABLE_WS.as_ptr(),
            UNREACHABLE_WS.len(),
            test_vtable(),
        )
    };
    let create_elapsed = t0.elapsed();
    assert!(!handle.is_null(), "create 应成功返回（不可达 host 也进泵）");
    assert!(
        create_elapsed < Duration::from_secs(2),
        "create 本身应秒级返回（connect 异步化，不在装配同步路径等 30s WS 连接），实际 {:?}",
        create_elapsed
    );

    // SAFETY: handle 来自 create，单次 destroy。
    unsafe { helix_destroy(handle) };
}

// ── ② 独立证伪：destroy 不被在途必达 Http 阻塞 ──────────────────────────────
//
// 关键：必须**确保命令已被 pump step 处理、必达 Http 已 submit 到 worker 并正在 reqwest 上挂起**
// 之后再 destroy——否则 destroy 时无在途 Http，drain 无须等网络，测不出 ② 的阻塞。
//
// 用录批回调等到 `im:post:sending` emit：它与 `Effect::Http`(POST /posts/create) **同一 step** 产出
// （module.rs handle_send_message 链）。收到该 emit ⇒ 该 step 已 dispatch 完成 ⇒ Http 已 submit
// 进 http_pool worker（连不上时 worker 正卡在 reqwest connect）。此刻 destroy → graceful drain，
// 若 drain 无上限（还原 `shutdown()`）则等满 reqwest timeout → FAIL；有上限（`shutdown_with_timeout`）
// 则 ≤ HTTP_DRAIN_LIMIT 退出 → PASS。

type Recorder = StdMutex<Vec<Vec<u8>>>;

extern "C" fn recording_events(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    // SAFETY: ctx 指向测试构造的 &Recorder（handle 存活期间有效）；ptr/len 由 FfiEventSink::flush
    // 传入，指向回调期间有效的 batch buffer。
    let rec = unsafe { &*(ctx as *const Recorder) };
    // SAFETY: ptr/len 由 FfiEventSink::flush 传入，指向回调期间有效的 batch buffer（立即拷贝，不持有）。
    let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Some(events) = helix_driver_ffi::batch::decode_batch(buf) {
        let mut g = rec.lock().unwrap();
        for e in events {
            g.push(e.to_vec());
        }
    }
}

/// 上界回归（② 独立）：在途必达 Http 挂起时，destroy 仍在 drain 上限内快速返回。
#[test]
fn destroy_not_blocked_by_inflight_mandatory_http() {
    let recorder: Box<Recorder> = Box::new(StdMutex::new(Vec::new()));
    let ctx = &*recorder as *const Recorder as *mut std::ffi::c_void;
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(recording_events),
        now_ms: Some(fixed_now_ms),
        ctx,
    };

    // SAFETY: 测试栈上有效指针/长度 + recorder ctx 在 handle 存活期间有效。
    let handle = unsafe {
        helix_create_with_network(
            UNREACHABLE_API.as_ptr(),
            UNREACHABLE_API.len(),
            UNREACHABLE_WS.as_ptr(),
            UNREACHABLE_WS.len(),
            vtable,
        )
    };
    assert!(!handle.is_null(), "create 应成功（不可达 host 也进泵）");

    // 发必达 Http 命令（im_send_message → Persist(乐观) + Http(POST /posts/create) + Timer + Emit）。
    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-inflight-http","text":"hi"}"#;
    // SAFETY: handle 来自 create，name/payload 指向测试栈上有效内存。
    let status = unsafe {
        helix_command(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
        )
    };
    assert_eq!(status, FFI_OK, "命令入队应成功");

    // 等到 im:post:sending emit 到达 ⇒ 该 step 已 dispatch ⇒ 必达 Http 已 submit 进 worker
    // （连不上时 worker 此刻正卡在 reqwest connect）。最多等 3s（emit 是同步上屏，毫秒级到达）。
    let mut http_inflight = false;
    for _ in 0..300 {
        let any_sending = recorder.lock().unwrap().iter().any(|e| {
            std::str::from_utf8(e)
                .map(|s| s.contains("im:post:sending"))
                .unwrap_or(false)
        });
        if any_sending {
            http_inflight = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        http_inflight,
        "应在超时内收到 im:post:sending（证明必达 Http 已 submit、正在 reqwest 上挂起）"
    );

    // 此刻在途必达 Http 正卡在 reqwest connect（连不上 blackhole）。destroy → graceful drain。
    let t_destroy = Instant::now();
    // SAFETY: handle 来自 create，单次 destroy。
    unsafe { helix_destroy(handle) };
    let destroy_elapsed = t_destroy.elapsed();

    // 端到端 sanity 上界（粗界，不做精算）：cap 机制的**确定性精证**在 host 层
    // `helix-driver-host/src/spawner_tests.rs::shutdown_with_timeout_caps_and_aborts_inflight`
    // （无网络/无墙钟 flake）。这里只验「destroy 没被等满整条网络 timeout」。
    //
    // 为什么界放到 20s 而非 4s（修 flake，HX-C011）：graceful drain 实为**两段顺序 cap**——
    // engine.rs 先 `http_pool.shutdown_with_timeout(3s)` 再 `http_fire_pool.shutdown_with_timeout(3s)`，
    // 结构上最坏 ~6s；且并行测试套件下多个 current_thread runtime 抢核 → tokio timer 饥饿，
    // 实测把 capped 路径拉到 ~10s（旧 4s 界 → 假阳 FAIL）。20s 仍**远小于** uncapped 回归
    // （还原 `shutdown()` 等满 reqwest = DEFAULT_TIMEOUT_SECS 30s）→ 稳分 capped(≤~10s 重载) vs
    // uncapped(30s)，不再 flaky。
    assert!(
        destroy_elapsed < Duration::from_secs(20),
        "destroy 不应被等满整条网络 timeout（drain 有上限 abort 在途；cap 精证见 host spawner_tests），实际 {:?}",
        destroy_elapsed
    );

    // recorder 在此 drop——destroy 已 join pump 线程，回调不再访问 ctx（无 use-after-free）。
    drop(recorder);
}
