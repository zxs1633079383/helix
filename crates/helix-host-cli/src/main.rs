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
    // HTTP api base 含 /api/cses 前缀：现网 cses-client imHttp base 即含此前缀，
    // helix-im to_effect 用 {api_base}/channels/load/increment、{api_base}/channel/sync/notify。
    // 真 Go 实测 `POST /api/cses/channels/load/increment` 返 200（B-rest 冒烟确认）。
    let api_base_url = std::env::var("HELIX_API_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:8065/api/cses".to_string());
    // cookieId(=userId) + companyId 身份头：现网 sync-v2 鉴权两件套。
    // companyId 经 web/handlers.go 注入 ctx → 传到对端 paas-grpc（loadTeamMap/ReadIdNameWithCompany）；
    // 缺 companyId → paas-grpc 业务失败（grpc service fail）→ increment worker 中止不推帧（真 Go 实测）。
    let cookie_id = std::env::var("HELIX_COOKIE_ID").unwrap_or_default();
    let company_id = std::env::var("HELIX_COMPANY_ID").unwrap_or_default();
    // appType 是现网 app 身份头（Apifox 实测：HTTP increment + WS 都带 `appType: bct`）。
    // 缺 appType → 对端 Java Company 服务判 session 过期 → grpc service fail → increment 不推帧。
    // device/deviceId/language 是 WS 全局头（Apifox WS 带 device:IOS/deviceId/language:zh）。
    let app_type = std::env::var("HELIX_APP_TYPE").unwrap_or_else(|_| "bct".to_string());
    let device = std::env::var("HELIX_DEVICE").unwrap_or_else(|_| "IOS".to_string());
    // deviceId 必须随 HTTP + WS 都带：对端 Java paas（/Users/mac28/workspace/java/x9/paas）
    // session 校验读 deviceId，nil → 判 session 过期 → grpc service fail → increment 不推帧（用户 debug 实证）。
    // 默认用测试设备 id（Apifox 同款）；生产由 host 按真实设备注入。
    let device_id = std::env::var("HELIX_DEVICE_ID")
        .unwrap_or_else(|_| "ED380414932542BAB4993E5AC3E07C64".to_string());
    let language = std::env::var("HELIX_LANGUAGE").unwrap_or_else(|_| "zh".to_string());
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
    // A3a：cookieId(=userId) 注入为 driver 全局 HTTP 身份头（现网 sync-v2 鉴权）。
    // 身份头由 host 注入，core/im 不认识 userId（connectionId 才由 im 盖章，见 A3b）。
    if !cookie_id.is_empty() {
        match http.set_global_header("cookieId", &cookie_id).await {
            Ok(()) => tracing::info!("已注入 cookieId 全局头（=userId）"),
            Err(e) => {
                tracing::warn!(error = %e, "注入 cookieId 全局头失败（继续，HTTP 将无身份头）")
            }
        }
    }
    // companyId HTTP 全局头：increment/sync 等所有 IM HTTP 必带，否则 paas-grpc 业务失败。
    if !company_id.is_empty() {
        match http.set_global_header("companyId", &company_id).await {
            Ok(()) => tracing::info!("已注入 companyId 全局头"),
            Err(e) => tracing::warn!(error = %e, "注入 companyId 全局头失败"),
        }
    }
    // appType HTTP 全局头：缺它对端 Java Company 服务判 session 过期 → grpc service fail。
    if !app_type.is_empty() {
        match http.set_global_header("appType", &app_type).await {
            Ok(()) => tracing::info!(app_type = %app_type, "已注入 appType 全局头"),
            Err(e) => tracing::warn!(error = %e, "注入 appType 全局头失败"),
        }
    }
    // deviceId HTTP 全局头：Java paas session 校验读它，nil → 判过期 → grpc service fail。
    if !device_id.is_empty() {
        match http.set_global_header("deviceId", &device_id).await {
            Ok(()) => tracing::info!("已注入 deviceId 全局头"),
            Err(e) => tracing::warn!(error = %e, "注入 deviceId 全局头失败"),
        }
    }
    // appType/device/language 也随 HTTP 带（与 WS 对齐，paas 可能校验多项）。
    if !device.is_empty() {
        let _ = http.set_global_header("device", &device).await;
    }
    if !language.is_empty() {
        let _ = http.set_global_header("language", &language).await;
    }
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
    // 现网 Go 在 WS 升级阶段按 cookieId=userId 建 session（session 空则不发 hello、~5s 超时关）。
    // 身份头必须随 upgrade 请求带，不能只设 HTTP 全局头（A3a 漏了 WS 这条，真 Go 冒烟暴露）。
    let mut ws_headers: Vec<(String, String)> = Vec::new();
    if !cookie_id.is_empty() {
        ws_headers.push(("cookieId".to_string(), cookie_id.clone()));
    }
    if !company_id.is_empty() {
        ws_headers.push(("companyId".to_string(), company_id.clone()));
    }
    // WS 全局头（Apifox 实测：appType/device/language/deviceId）
    if !app_type.is_empty() {
        ws_headers.push(("appType".to_string(), app_type.clone()));
    }
    if !device.is_empty() {
        ws_headers.push(("device".to_string(), device.clone()));
    }
    if !language.is_empty() {
        ws_headers.push(("language".to_string(), language.clone()));
    }
    if !device_id.is_empty() {
        ws_headers.push(("deviceId".to_string(), device_id.clone()));
    }
    let mut transport = NativeTransport::new(ws_url.clone(), main_transport, Some(tick_tx.clone()))
        .with_handshake_headers(ws_headers);
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
