//! FfiClock — host 注入壁钟（确定性，GOAL §1.2 / HX-C002）。
//!
//! core 的 `step()` 前壁钟由 host 提供：pump 调 `Clock::now_ms()` → 转调
//! `CppHostVtable.now_ms(ctx)`。**host = C++ shim**（与 pump 同进程、回调线程内同步返回 epoch ms），
//! 非 Dart——确定性注入靠此链路成立（GOAL §1.2）。host 不提供（`now_ms == None`）时 fallback
//! `SystemTime` 并 warn——fallback 破坏「时间全由 host 注入」的确定性，仅作健壮性兜底，生产应填 vtable。
//! 测试用 mock `now_ms` fn 跑确定性回放（同输入同输出）。

use std::ffi::c_void;

use helix_core::ports::Clock;

/// FFI 时钟：把 core 的时间需求转调 C++ host 的 `now_ms` 回调。
///
/// 仅持 `extern "C" fn` 指针 + `*mut c_void` ctx（均 Copy），故 `Clone` 廉价。
/// `ctx` 不解引用，仅原样回传给回调（与 [`crate::vtable::CppHostVtable`] 同契约）。
#[derive(Clone, Copy)]
pub struct FfiClock {
    now_ms_fn: Option<extern "C" fn(ctx: *mut c_void) -> u64>,
    ctx: *mut c_void,
}

// SAFETY: 只持函数指针（代码地址，线程无关）+ 不透明 ctx（Rust 不解引用，host 自管同步）。
// 与 CppHostVtable 的 Send/Sync 论证同源——pump 在专用线程跑，需 Clock: MaybeSend(=Send on native)。
unsafe impl Send for FfiClock {}
// SAFETY: 同上，无内部可变状态。
unsafe impl Sync for FfiClock {}

impl FfiClock {
    /// 从 vtable 的 `now_ms` + `ctx` 构造（vtable 整体 Copy，调用方传副本即可）。
    pub fn new(
        now_ms_fn: Option<extern "C" fn(ctx: *mut c_void) -> u64>,
        ctx: *mut c_void,
    ) -> Self {
        Self { now_ms_fn, ctx }
    }
}

impl Clock for FfiClock {
    fn now_ms(&self) -> u64 {
        match self.now_ms_fn {
            // now_ms 是安全 fn 指针（非 unsafe extern），直接调用；host 契约保证其在 destroy 返回前
            // 有效（vtable.rs ctx 生命周期），ctx 原样回传不解引用。
            Some(f) => f(self.ctx),
            None => {
                use std::time::{SystemTime, UNIX_EPOCH};
                // fallback：host 未提供 now_ms → 用进程壁钟。破坏确定性注入，仅健壮性兜底。
                tracing::warn!(
                    "CppHostVtable.now_ms 为 None，fallback SystemTime（确定性注入失效）"
                );
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }
        }
    }
}
