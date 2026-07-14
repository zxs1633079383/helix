//! helix-host-cli native composition root：装配 core、IM、共享 host I/O 与显式 OTel hooks。
//! 默认读取 `./helix.config.yaml`；`--config` 与双账号 identity flags 见 `config.rs`。

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

mod cmd_parse;
mod config;
mod host_env;
mod repl;
use host_env::{default_api_base, machine_device_id, resolve_db_path, DbPathInputs};

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::TraceHooks;
use helix_driver_native::{
    run_engine_loop_with_transports_and_trace, AuthTokenRegistry, EngineConfig, HostNetworkConfig,
    NativeEventSink, NativeHttp, NativeStorage, NativeTransport, SharedFileUploader,
    SharedHttpClient, TransportTable,
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

    // YAML 为主真源；debug env 只覆盖非 OTel 字段，identity flags 最高优先。
    let cli = config::parse_args(std::env::args().skip(1));
    let mut cfg = match config::Config::load(&cli.config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config_path,
                "读配置失败，回退内置默认（指 cses-im-server :8066）");
            config::Config::default()
        }
    };
    cfg.apply_env_overrides(); // debug 逃生口（release 为 no-op，彻底去 env）
    cfg.apply_cli_overrides(&cli); // 双账号 e2e：flag 覆盖 identity（最高优先级）
    let otel = cfg.observability.otel_runtime();

    let ws_url = cfg.connection.ws_url.clone();
    // api base 含 /api/cses；default_api_base 承载 vote/score 第二网关。
    let api_base_url = cfg.connection.api_base.clone();
    let default_api_base_url = default_api_base(&api_base_url, &cfg.connection.default_api_base);
    // cookieId(=userId) + companyId 是 sync-v2/paas-grpc 鉴权两件套。
    let cookie_id = cfg.identity.cookie_id.clone();
    let company_id = cfg.identity.company_id.clone();
    // appType 是现网 app 身份头（HTTP increment + WS 都带 `appType: bct`）。缺 → Java 判 session 过期。
    let app_type = cfg.client.app_type.clone();
    let device = cfg.client.device.clone();
    // deviceId 优先 config；ADR-011 联调兜底仅属于此 dev-bin，不进入交付物。
    let device_id = (!cfg.client.device_id.is_empty())
        .then(|| cfg.client.device_id.clone())
        // ADR-011 故意例外：固定联调 deviceId（本机派生过不了 Go session 校验，非生产凭据）。
        .or_else(|| Some("ED380414932542BAB4993E5AC3E07C64".to_string()))
        .or_else(machine_device_id)
        .unwrap_or_default();
    if device_id.is_empty() {
        tracing::warn!(
            "未能派生 deviceId（config client.device_id 可设）；HTTP/WS 将不带 deviceId，对端可能判 session 过期"
        );
    } else {
        tracing::info!(device_id_present = true, "deviceId 已解析（config / 派生）");
    }
    let language = cfg.client.language.clone();
    // appVersion/osVersion 是现网通用请求头（真机curl真源 §0：所有 outbound 都带）。
    let app_version = cfg.client.app_version.clone();
    let os_version = cfg.client.os_version.clone();
    // userSnapshot 由 host 展开注入，core/im 不依赖 ucenter（HX-C001）。
    let user_name = cfg.identity.user_name.clone();
    let org_name = cfg.identity.org_name.clone();
    let dept_name = cfg.identity.dept_name.clone();
    // ── DB 路径派生（多租户，对齐现网 678 真库目录）───────────────────────
    // 优先级：runtime.db 显式覆盖 > 派生路径 > /tmp 兜底。
    // 派生形态：~/Library/Application Support/com.jinqidongli.cses.dev/data/<store-key>/tenant.db
    // <store-key> 由 app/backend/company/account/schema 归一后 hash 得出。
    let db_path = resolve_db_path(DbPathInputs {
        cookie_id: &cookie_id,
        company_id: &company_id,
        configured_db: &cfg.runtime.db,
        configured_tenant: &cfg.identity.tenant,
        api_base_url: &api_base_url,
        ws_url: &ws_url,
    });

    tracing::info!(
        %ws_url,
        %api_base_url,
        cookie_id_present = !cookie_id.is_empty(),
        "helix-host-cli 启动（Phase A1：验证 Effect::Send 上行）"
    );

    // ── ports ───────────────────────────────────────────────
    tracing::info!(db_path = %db_path, "DB 绝对路径（派生 / runtime.db 覆盖）");
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
        let total = helix_im::IM_SCHEMA.len() + helix_im::IM_SCHEMA_MIGRATIONS.len();
        for sql in helix_im::IM_SCHEMA {
            match storage.execute_raw(sql).await {
                Ok(()) => ok += 1,
                Err(e) => tracing::error!(error = %e, "建 IM 表失败（继续尝试其余语句）"),
            }
        }
        for sql in helix_im::IM_SCHEMA_MIGRATIONS {
            match storage.execute_raw(sql).await {
                Ok(()) => ok += 1,
                Err(e) if is_duplicate_column_error(&e) => ok += 1,
                Err(e) => tracing::error!(error = %e, "迁移 IM 表失败（继续尝试其余语句）"),
            }
        }
        if ok == total {
            tracing::info!(statements = total, "IM schema 建表完成（全部成功）");
        } else {
            tracing::warn!(ok, total, "IM schema 建表部分失败");
        }
    }
    // EventSink 先于 HTTP：P6 blocker(b) 横切层 401/断网回报经它 emit，故 with_crosscut 前先建。
    // Clone 是 Arc-backed O(1)：一份给横切回报，原件随后进 EngineConfig（同一 broadcast）。
    let (event_sink, mut event_rx) = NativeEventSink::new();

    // 横切层负责鉴权意图、401/断网事件与 trace；凭据留 driver，core 零 token（C6）。
    // 通用身份头仍由下方共享 registry 注入。
    let auth = AuthTokenRegistry::new();
    if !cookie_id.is_empty() {
        auth.set_session("cookieId", cookie_id.clone());
    }
    let mut net_config = HostNetworkConfig::new(api_base_url.clone(), ws_url.clone());
    net_config.default_api_base_url = default_api_base_url.clone();
    let shared_client = match SharedHttpClient::new(net_config) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "构造共享 HTTP client 失败，退出");
            return;
        }
    };
    let http = NativeHttp::with_crosscut_and_otel(
        shared_client,
        Arc::new(event_sink.clone()),
        auth,
        otel.clone(),
    );
    // HTTP/WS 复用同一份 host 身份头；缺 cookie/company/device 会触发服务端鉴权失败。
    // 空值不编造，connectionId 仍由 IM 盖章。
    let identity_headers: Vec<(&str, &str)> = [
        ("cookieId", cookie_id.as_str()),
        ("companyId", company_id.as_str()),
        ("appType", app_type.as_str()),
        ("device", device.as_str()),
        ("language", language.as_str()),
        ("deviceId", device_id.as_str()),
        ("appVersion", app_version.as_str()),
        ("osVersion", os_version.as_str()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .collect();
    for (name, value) in &identity_headers {
        match http.set_global_header(name, value).await {
            Ok(()) => tracing::info!(header = %name, "已注入 HTTP 全局头"),
            Err(e) => tracing::warn!(error = %e, header = %name, "注入 HTTP 全局头失败（继续）"),
        }
    }

    // ── shell + IM 模块 ─────────────────────────────────────
    let mut shell = ExecutionShell::new();
    // auth_user_id = cookieId(=userId)：host 注入确定性身份；空串退化「自己消息也计未读」（§5，todo #5）。
    if cookie_id.is_empty() {
        tracing::warn!(
            "identity.cookie_id 为空 → ImConfig.auth_user_id 为空，post_updates 将退化为「自己消息也计未读」（离线未读偏高）"
        );
    }
    shell.register(ImModule::new(ImConfig {
        ws_url: ws_url.clone(),
        api_base_url,
        default_api_base_url, // spec06 方案 A：vote/score 第二网关
        auth_user_id: cookie_id.clone(),
        // companyId 之前只灌了 HTTP/WS 全局头，没进 ImConfig；现注入 → outbound posts/create
        // body 的 teamId/userSnapshot.teamId 才有真值（真机curl真源 §1，send 回声前置条件）。
        company_id: company_id.clone(),
        // P6 blocker(a)：userSnapshot 三占位转真值（ucenter 展开，host 注入，HX-C001）。
        user_name: user_name.clone(),
        org_name: org_name.clone(),
        dept_name: dept_name.clone(),
        ..Default::default()
    }));

    // ── tick 通道（入站有界 cap=256）────────────────────────
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);

    // ── 主 WS transport ─────────────────────────────────────
    // TransportId 约定：0 = 主连接（多端口/第二模块用 1、2…，host 集中分配）。
    let main_transport = TransportId::from_raw(0);
    // WS upgrade 必须携带同一份身份头，否则服务端不发 hello。
    // 复用 HTTP identity_headers，避免两个配置源漂移。
    let ws_headers: Vec<(String, String)> = identity_headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
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
    // host-cli 走「预填表」路径：上面已同步 connect 好，直接放共享句柄进表（去外层 Mutex，
    // 对齐 send/recv/close 的 `&self`）。FFI 端因 create 不能 await connect 才用注册通道。
    let mut transports = TransportTable::new();
    transports.insert(main_transport, Arc::new(transport));

    // Http 并发上限：runtime.http_max_inflight（0/缺省 → 默认 8）。
    let max_http_inflight = if cfg.runtime.http_max_inflight > 0 {
        cfg.runtime.http_max_inflight
    } else {
        8
    };
    tracing::info!(
        max_http_inflight,
        "Http 并发闸 N（runtime.http_max_inflight）"
    );

    let config = EngineConfig {
        storage,
        http,
        uploader: SharedFileUploader::default(),
        event_sink,
        metrics: Arc::new(helix_driver_host::NoopMetricSink),
        max_http_inflight,
    };
    let trace = TraceHooks::noop().with_otel(otel);

    // ── 观察 emit 出口（领域事件打印，验证 EventSink 兑现）──
    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                // 解出事件名供 e2e 断言；解析失败回退长度，不崩。
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
    // stdin 命令复用 tick_tx；quit/EOF 触发优雅退出。
    repl::spawn_stdin_repl(tick_tx.clone(), shutdown_tx);
    let tick_tx_for_loop = tick_tx.clone();
    tracing::info!("进入 engine loop（stdin 遥控器已就绪，敲 help 看命令）");
    run_engine_loop_with_transports_and_trace(
        shell,
        tick_rx,
        tick_tx_for_loop,
        config,
        trace,
        shutdown_rx,
        transports,
    )
    .await;
    tracing::info!("engine loop 退出");
}

fn is_duplicate_column_error(e: &impl std::fmt::Display) -> bool {
    e.to_string().contains("duplicate column name")
}
