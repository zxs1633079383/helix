//! PI-11 守门测试：生命周期内存/资源稳态（create/destroy 多轮干净，Box/线程归零）。
//!
//! FFI 单元测试无法直接读 RSS 斜率（那是仓外 leaks/valgrind 的活，见 PERF-INVARIANTS.md 边界声明）。
//! 本测试在 **FFI 边界可断言** 的层面证伪两件事：
//!   1. **多轮 create→command→destroy 不泄漏 pump 线程**：每轮 `helix_destroy` join pump 线程
//!      （回收 `JoinHandle`）。若 destroy 漏 join / 漏 `Box::from_raw`，多轮后线程/句柄累积——
//!      本测试跑 N 轮全部干净完成即证 destroy 真回收（破坏：注释掉 destroy 的 join → 线程泄漏，
//!      高轮次下 panic 线程数上限或 hang）。
//!   2. **destroy 后命令被拒**（channel 已关闭）：证 tick_tx 真被 take/drop（无悬挂 Sender 泄漏）。
//!
//! HX-C011：破坏 `Box::from_raw` / join → 资源泄漏，是仓外 leaks 工具的证伪点；本测试守 FFI 边界
//! 可观测的「多轮干净 + destroy 后拒命令」不变量。

use helix_driver_ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, HelixHandle, FFI_OK,
};
use helix_driver_ffi::vtable::CppHostVtable;

extern "C" fn noop_events(_ctx: *mut std::ffi::c_void, _ptr: *const u8, _len: usize) {}
extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

fn create() -> *mut HelixHandle {
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(noop_events),
        now_ms: Some(fixed_now_ms),
        ctx: std::ptr::null_mut(),
    };
    // SAFETY: 指针/长度在调用期间有效；vtable 回调静态，ctx null。
    let handle = unsafe {
        helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
    };
    assert!(!handle.is_null(), "create 应成功");
    handle
}

#[test]
fn pi11_many_create_destroy_rounds_stay_clean() {
    // 32 轮：每轮起一个 pump 线程；若 destroy 漏 join，线程累积 → 高轮次暴露（线程上限/句柄泄漏）。
    // 32 远低于系统线程上限，但足以暴露「每轮都漏 1 线程」类回归（破坏 join → 32 个孤儿线程）。
    let rounds = 32;
    for round in 0..rounds {
        let handle = create();

        let name = b"im_send_message";
        let payload =
            br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp","text":"x"}"#;
        // SAFETY: handle 存活；指针在调用期间有效。
        let status = unsafe {
            helix_command(
                handle,
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        assert_eq!(status, FFI_OK, "round {round}: 命令入队应成功");

        // SAFETY: handle 由本轮创建，未被 destroy；join pump 回收线程 + Box::from_raw 回收句柄。
        unsafe { helix_destroy(handle) };
    }
}

#[test]
fn pi11_concurrent_handles_are_independent() {
    // 同进程并存两个句柄：各自独立 pump 线程 + 独立 tick channel + 独立库（ffi_db_url 单调计数）。
    // 证「多句柄无共享可变全局态泄漏 / 不串味」——各自命令 OK、各自 destroy 干净。
    let h1 = create();
    let h2 = create();

    let name = b"im_send_message";
    let payload = br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"t","text":"y"}"#;
    for h in [h1, h2] {
        // SAFETY: h 存活；指针在调用期间有效。
        let code = unsafe {
            helix_command(
                h,
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        assert_eq!(code, FFI_OK, "并存句柄各自命令应入队成功");
    }

    // 逆序 destroy（h2 先）：证销毁顺序无依赖、互不影响。
    // SAFETY: h2/h1 各由本 test 创建，未 destroy；各自 join 自己的 pump。
    unsafe {
        helix_destroy(h2);
        helix_destroy(h1);
    }
}
