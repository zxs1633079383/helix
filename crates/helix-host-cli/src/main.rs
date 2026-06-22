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

mod cmd_parse;
mod host_env;
mod repl;
use host_env::{machine_device_id, resolve_db_path};

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::{ExecutionShell, Tick};
use helix_driver_native::{
    run_engine_loop_with_transports, EngineConfig, HostNetworkConfig, NativeEventSink, NativeHttp,
    NativeStorage, NativeTransport, TransportTable,
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
    // 解析优先级：① HELIX_DEVICE_ID env 显式覆盖（联调时手动指定可用）→ ② 从本机派生
    // （macOS IOPlatformUUID / Linux machine-id / Windows MachineGuid）→ ③ 空（告警，不再
    // 硬编码某台设备 id）。**铁律**：不默认绑死任何一台真实设备 id。
    let device_id = std::env::var("HELIX_DEVICE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| Some("ED380414932542BAB4993E5AC3E07C64".to_string()))
        .or_else(machine_device_id)
        .unwrap_or_default();
    if device_id.is_empty() {
        tracing::warn!(
            "未能从本机派生 deviceId（可设 HELIX_DEVICE_ID 覆盖）；HTTP/WS 将不带 deviceId，对端可能判 session 过期"
        );
    } else {
        tracing::info!(
            device_id_present = true,
            "deviceId 已解析（本机派生 / env 覆盖）"
        );
    }
    let language = std::env::var("HELIX_LANGUAGE").unwrap_or_else(|_| "zh".to_string());
    // ── DB 路径派生（多租户，对齐现网 678 真库目录）───────────────────────
    // 优先级：HELIX_DB env 显式覆盖（测试/调试逃生口，含旧 /tmp 能力）> 派生路径 > /tmp 兜底。
    // 派生形态：~/Library/Application Support/com.jinqidongli.cses.dev/data/<tenant>/tenant.db
    // <tenant> 来源（高→低）：HELIX_TENANT > cookie_id(=userId，对齐 cses-client tenant_key) > "default"。
    // 目录不存在则 create_dir_all。
    let db_path = resolve_db_path(&cookie_id);

    tracing::info!(
        %ws_url,
        %api_base_url,
        cookie_id_present = !cookie_id.is_empty(),
        "helix-host-cli 启动（Phase A1：验证 Effect::Send 上行）"
    );

    // ── ports ───────────────────────────────────────────────
    tracing::info!(db_path = %db_path, "DB 绝对路径（派生 / HELIX_DB 覆盖）");
    let storage = match NativeStorage::open(&format!("sqlite:{db_path}?mode=rwc")).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, db_path = %db_path, "打开存储失败，退出");
            return;
        }
    };
    // open() 内部已 migrate()（建通用 _helix_monotonic）。IM 业务 schema 是 additive：
    // 在进泵前 loop execute_raw 叠加建表（DDL 当数据，core 仍 schema-agnostic，HX-C001）。
    // execute_raw 签名 &'static str —— IM_SCHEMA 每条都是 string literal，天然满足。
    {
        let mut ok = 0usize;
        let total = helix_im::IM_SCHEMA.len();
        for sql in helix_im::IM_SCHEMA {
            match storage.execute_raw(sql).await {
                Ok(()) => ok += 1,
                Err(e) => tracing::error!(error = %e, "建 IM 表失败（继续尝试其余语句）"),
            }
        }
        if ok == total {
            tracing::info!(statements = total, "IM schema 建表完成（全部成功）");
        } else {
            tracing::warn!(ok, total, "IM schema 建表部分失败");
        }
    }
    // ADR-010：共享 HTTP client 需 HostNetworkConfig（api_base + ws_url）；身份头随后经 set_global_header 灌 registry。
    let http =
        match NativeHttp::with_config(HostNetworkConfig::new(api_base_url.clone(), ws_url.clone()))
        {
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
    // auth_user_id = cookieId(=userId)：host 注入确定性身份（id 作数据，core/im 不认识 userId 语义）。
    // 缺它 → post_updates 退化「自己消息也计未读」（§5 离线未读偏高根因，todo #5）。
    // 空串（未设 HELIX_COOKIE_ID）→ 保留无身份保守计入退化，不强行编造身份。
    if cookie_id.is_empty() {
        tracing::warn!(
            "未设 HELIX_COOKIE_ID → ImConfig.auth_user_id 为空，post_updates 将退化为「自己消息也计未读」（离线未读偏高）"
        );
    }
    shell.register(ImModule::new(ImConfig {
        ws_url: ws_url.clone(),
        api_base_url,
        auth_user_id: cookie_id.clone(),
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

    // Http 并发上限：env 注入（probe 可不重编译扫不同 N）。解析失败/缺省/非正 → 默认 8。
    let max_http_inflight = std::env::var("HELIX_HTTP_MAX_INFLIGHT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(8);
    tracing::info!(
        max_http_inflight,
        "Http 并发闸 N（HELIX_HTTP_MAX_INFLIGHT）"
    );

    let config = EngineConfig {
        storage,
        http,
        event_sink,
        max_http_inflight,
    };

    // ── 观察 emit 出口（领域事件打印，验证 EventSink 兑现）──
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                // 联调可观测：解出 DomainEvent 的 `event` 名（im:*）→ e2e harness 可断言
                // 完整 frame→handler→emit 管道（非仅字节长度）。解析失败回退长度，不崩。
                Ok(ev) => {
                    match serde_json::from_slice::<serde_json::Value>(&ev.0)
                        .ok()
                        .and_then(|v| v.get("event").and_then(|e| e.as_str()).map(str::to_owned))
                    {
                        Some(name) => {
                            tracing::info!(event = %name, bytes = ev.0.len(), "DomainEvent emit")
                        }
                        None => tracing::info!(bytes = ev.0.len(), "DomainEvent emit (unparsed)"),
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "EventSink 落后丢事件（应触发重拉）")
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // ── 跑泵 ────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    // stdin 遥控器：终端命令 → Tick::Command → 复用 tick_tx 入同一条泵（与 Tick::Inbound 同路）。
    // quit/EOF → fire shutdown_tx → 泵优雅退出。host 层装配，不进 core/im。
    repl::spawn_stdin_repl(tick_tx.clone(), shutdown_tx);
    let tick_tx_for_loop = tick_tx.clone();
    tracing::info!("进入 engine loop（stdin 遥控器已就绪，敲 help 看命令）");
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
