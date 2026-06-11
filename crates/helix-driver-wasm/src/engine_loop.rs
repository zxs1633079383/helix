//! wasm engine_loop — wasm_bindgen_futures::spawn_local 事件泵
//!
//! spawn_local 在此，不在 helix-core。
//! 单线程事件循环（wasm32 无多线程），与 native tokio::select! 泵行为等价。

use helix_core::ExecutionShell;

/// 运行 ExecutionShell 的 wasm 事件泵。
///
/// 通过 `wasm_bindgen_futures::spawn_local` 在 wasm 单线程上调度。
#[cfg(target_arch = "wasm32")]
pub fn run_engine_loop(shell: ExecutionShell) {
    wasm_bindgen_futures::spawn_local(async move {
        run_engine_loop_inner(shell).await;
    });
}

#[cfg(target_arch = "wasm32")]
async fn run_engine_loop_inner(mut shell: ExecutionShell) {
    // TODO: 与 native engine_loop 对称的 futures::select! 循环
    // 事件源：WS message callback → channel → select
    //         gloo_timers::future::TimeoutFuture → timer fired
    //         JS -> wasm 命令注入 → channel
    let _ = shell;
    tracing::info!("helix wasm engine loop started (TODO)");
}

/// 非 wasm 目标的空实现（让 cargo check 通过）
#[cfg(not(target_arch = "wasm32"))]
pub fn run_engine_loop(_shell: ExecutionShell) {
    panic!("run_engine_loop is only for wasm32 target");
}
