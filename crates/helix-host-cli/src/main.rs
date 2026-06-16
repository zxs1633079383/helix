//! helix-host-cli — 最小 native 宿主（sync-v2 Phase A1）
//!
//! 把 `ExecutionShell` + `ImModule` + `NativeTransport` 接成端到端可跑闭环，
//! 验证 `Effect::Send` 真正发到 WS（解锁「真连 Go」的上行半边）。
//!
//! ## A1 临时选择（见设计文档 §7 待决）
//! - **host 主动 connect** 主 WS（`TransportId(0)`）。connect 触发模型
//!   （`Effect::Connect` 意图 vs host 主动）留 **A2** 决策。
//! - `cookieId`(=userId) 当前**只读不注入** header；身份头注入留 **A3**。
//! - 入站 WS 帧 → `Tick::Inbound` 的 recv 接线留 **A2**（本 bin 只证上行）。
//!
//! ## 用法
//! ```bash
//! HELIX_WS_URL=ws://127.0.0.1:8065/api/v4/websocket \
//! HELIX_COOKIE_ID=5ffd465e263864787b87aa01 \
//! cargo run -p helix-host-cli
//! ```

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::{ExecutionShell, Tick};
use helix_driver_native::{
    run_engine_loop_with_transports, EngineConfig, NativeEventSink, NativeHttp, NativeStorage,
    NativeTransport, TransportTable,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use tokio::sync::broadcast::error::RecvError;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let ws_url = std::env::var("HELIX_WS_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:8065/api/v4/websocket".to_string());
    let api_base_url =
        std::env::var("HELIX_API_BASE").unwrap_or_else(|_| "http://127.0.0.1:8065".to_string());
    // A1：cookieId(=userId) 先读不强制用；A3 注入为 HTTP/WS 身份头
    let cookie_id = std::env::var("HELIX_COOKIE_ID").unwrap_or_default();
    let db_path =
        std::env::var("HELIX_DB").unwrap_or_else(|_| "/tmp/helix_host_cli.db".to_string());

    tracing::info!(
        %ws_url,
        %api_base_url,
        cookie_id_present = !cookie_id.is_empty(),
        "helix-host-cli 启动（Phase A1：验证 Effect::Send 上行）"
    );

    // ── ports ───────────────────────────────────────────────
    let storage = match NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc")).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "打开存储失败，退出");
            return;
        }
    };
    let http = match NativeHttp::new() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "构造 HTTP 客户端失败，退出");
            return;
        }
    };
    let (event_sink, mut event_rx) = NativeEventSink::new();

    // ── shell + IM 模块 ─────────────────────────────────────
    let mut shell = ExecutionShell::new();
    shell.register(ImModule::new(ImConfig {
        ws_url: ws_url.clone(),
        api_base_url,
        ..Default::default()
    }));

    // ── tick 通道（入站有界 cap=256）────────────────────────
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);

    // ── 主 WS transport ─────────────────────────────────────
    // TransportId 约定：0 = 主连接（多端口/第二模块用 1、2…，host 集中分配）。
    let main_transport = TransportId::from_raw(0);
    let mut transport = NativeTransport::new(ws_url.clone(), main_transport, Some(tick_tx.clone()));
    // A1 临时：host 主动 connect（A2 改为 Effect::Connect 意图驱动）。
    // 连不上不退出——仍进泵，Effect::Send 走「发送失败 warn」兜底，证明接线存在。
    match transport.connect().await {
        Ok(()) => tracing::info!(transport_id = main_transport.raw(), "主 WS 已连接"),
        Err(e) => tracing::error!(
            error = %e,
            "主 WS 连接失败（Go 未起？）——仍进泵，上行 Send 将走 warn 兜底"
        ),
    }
    let mut transports = TransportTable::new();
    transports.insert(main_transport, Arc::new(Mutex::new(transport)));

    let config = EngineConfig {
        storage,
        http,
        event_sink,
    };

    // ── 观察 emit 出口（领域事件打印，验证 EventSink 兑现）──
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(ev) => tracing::info!(bytes = ev.0.len(), "DomainEvent emit"),
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "EventSink 落后丢事件（应触发重拉）")
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // ── 跑泵 ────────────────────────────────────────────────
    let (_shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let tick_tx_for_loop = tick_tx.clone();
    tracing::info!("进入 engine loop");
    run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx_for_loop,
        config,
        shutdown_rx,
        transports,
    )
    .await;
    tracing::info!("engine loop 退出");
}
