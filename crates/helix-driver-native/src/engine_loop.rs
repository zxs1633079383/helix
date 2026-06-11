//! engine_loop — tokio 事件泵
//!
//! ## tokio::spawn 在此，不在 helix-core
//!
//! helix-core 保持零运行时依赖。
//! 所有异步协调（WS recv、HTTP 响应、timer fired、Tauri 命令）在此 tokio select! 循环。
//!
//! ## 单 ExecutionShell 设计
//!
//! 默认：单个 ExecutionShell + 单个 tokio::select! 泵（串行处理所有 Tick）。
//! 可选：多 shell hash 分片（hash(channel_id) % N，每 shard 独立 JoinHandle），
//!       这是 native driver 的实现细节，helix-core 和 helix-im 不知道此决策。

use helix_core::ExecutionShell;
use helix_core::ports::Clock;
use crate::clock::NativeClock;

/// 运行 ExecutionShell 的 tokio 事件泵。
///
/// 调用者（helix-tauri 的 setup.rs）负责创建 shell + 注册模块后调用此函数。
pub async fn run_engine_loop(mut shell: ExecutionShell) {
    let clock = NativeClock;

    // 初始化：触发 on_start
    match shell.start() {
        Ok(effects) => {
            // TODO: 兑现初始 effects（连接 WS、读 cursor 等）
            tracing::info!("helix engine started, {} initial effects", effects.len());
        }
        Err(e) => {
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    }

    // TODO: 事件泵主循环
    // tokio::select! {
    //     // WS 入站帧
    //     Some(frame) = ws_rx.recv() => {
    //         let tick = Tick::Inbound(InboundBytes(&frame));
    //         let effects = shell.step(tick, clock.now_ms()).unwrap();
    //         execute_effects(effects).await;
    //     }
    //     // HTTP 响应（PortReply）
    //     Some((corr, outcome)) = http_result_rx.recv() => {
    //         let tick = Tick::PortReply { corr, outcome };
    //         let effects = shell.step(tick, clock.now_ms()).unwrap();
    //         execute_effects(effects).await;
    //     }
    //     // Timer 触发
    //     Some(timer_id) = timer_rx.recv() => {
    //         let tick = Tick::Timer(timer_id);
    //         let effects = shell.step(tick, clock.now_ms()).unwrap();
    //         execute_effects(effects).await;
    //     }
    //     // Tauri 命令（UI 注入）
    //     Some(cmd) = cmd_rx.recv() => {
    //         let tick = Tick::Command(cmd);
    //         let effects = shell.step(tick, clock.now_ms()).unwrap();
    //         execute_effects(effects).await;
    //     }
    // }
}
