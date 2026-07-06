//! helix-host-cli — 最小 native 宿主（sync-v2 Phase A1）
//!
//! 把 `ExecutionShell` + `ImModule` + `NativeTransport` 接成端到端可跑闭环，
//! 验证 `Effect::Send` 真正发到 WS（解锁「真连 Go」的上行半边）。
//!
//! ## 用法（去 env 化，PRD §4.1/§4.2）
//! ```bash
//! # 默认读 repo 根 ./helix.config.yaml（指 cses-im-server :8066）
//! cargo run -p helix-host-cli
//! # 自定义路径 + 双账号 e2e 身份覆盖
//! cargo run -p helix-host-cli -- --config helix.config.yaml --cookie-id 444 --company-id 64118...
//! ```
//! debug 构建保留 `HELIX_<FIELD>` env 逃生口（覆盖对应 config 值，不文档化）。

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
use helix_driver_native::{
    run_engine_loop_with_transports, AuthTokenRegistry, EngineConfig, HostNetworkConfig,
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

    // ── 配置加载（去 env 化，PRD §4.1/§4.2）─────────────────────────────
    // 单一 YAML 真源 helix.config.yaml（默认 repo 根，--config 覆盖路径）。覆盖链（低→高）：
    // YAML < HELIX_<FIELD> env 逃生口（仅 debug 构建，release no-op）< --cookie-id/--company-id flag。
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

    let ws_url = cfg.connection.ws_url.clone();
    // HTTP api base 含 /api/cses 前缀（helix-im to_effect 拼 channels/load/increment 等）。
    let api_base_url = cfg.connection.api_base.clone();
    // spec06 方案 A：vote/score 第二网关（connection.default_api_base 空则派生）。
    let default_api_base_url = default_api_base(&api_base_url, &cfg.connection.default_api_base);
    // cookieId(=userId) + companyId 身份头：现网 sync-v2 鉴权两件套。
    // companyId 经 web/handlers.go 注入 ctx → 传到对端 paas-grpc（loadTeamMap/ReadIdNameWithCompany）；
    // 缺 companyId → paas-grpc 业务失败（grpc service fail）→ increment worker 中止不推帧（真 Go 实测）。
    let cookie_id = cfg.identity.cookie_id.clone();
    let company_id = cfg.identity.company_id.clone();
    // appType 是现网 app 身份头（HTTP increment + WS 都带 `appType: bct`）。缺 → Java 判 session 过期。
    let app_type = cfg.client.app_type.clone();
    let device = cfg.client.device.clone();
    // deviceId 随 HTTP + WS 都带：对端 Java paas session 校验读 deviceId，nil → 判 session 过期。
    // 优先级：① config.client.device_id 显式 → ② 固定联调 deviceId 兜底（**故意例外** ADR-011：
    // 本机派生过不了 Go session 校验，仅此 dev-bin、不入交付物）→ ③ 本机派生 → ④ 空告警。
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
    // P6 blocker(a)：userSnapshot 三占位转真值（现网取自 ucenter `this.ctx.userInfo`，helix-im
    // 不认识 ucenter HX-C001 → host 展开注入 ImConfig；host-cli 走 config，Tauri 壳走登录态）。
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
    // EventSink 先于 HTTP：P6 blocker(b) 横切层 401/断网回报经它 emit，故 with_crosscut 前先建。
    // Clone 是 Arc-backed O(1)：一份给横切回报，原件随后进 EngineConfig（同一 broadcast）。
    let (event_sink, mut event_rx) = NativeEventSink::new();

    // P6 blocker(b)：激活 P1.5 出站横切层（drift-review dbe1b74..7e9f3ca finding① 收敛=装配就绪
    // 未接泵→此处接泵）。with_crosscut 包：鉴权意图→token 注入 + 401→im:http:unauthorized +
    // 断网→im:net:offline + traceparent 透传。通用身份头仍走下方 set_global_header（registry 句柄保留）。
    // session 凭据 = cookieId(=userId)（现网 IM_HTTP session 轴）；实际 token 留 driver，core 零 token(C6)。
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
    let http = NativeHttp::with_crosscut(shared_client, Arc::new(event_sink.clone()), auth);
    // 身份头单一真源（HTTP 全局头 + WS 握手头同源，host 注入；core/im 不认识 userId，
    // connectionId 才由 im 盖章见 A3b）。各头来源/缺失后果（真机curl真源 §0 通用头 +
    // sync-v2 鉴权）：cookieId(=userId)/companyId 缺→paas-grpc 业务失败；appType 缺→Java
    // 判 session 过期；deviceId 缺→paas session 校验过期；appVersion/osVersion 通用头。
    // 空值项跳过（不编造）。`&str` slice 一次构造，两处（HTTP/WS）复用，去重 + 守 ≤300 行。
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
    // 现网 Go 在 WS 升级阶段按 cookieId=userId 建 session（session 空则不发 hello、~5s 超时关）。
    // 身份头必须随 upgrade 请求带，不能只设 HTTP 全局头（A3a 漏了 WS 这条，真 Go 冒烟暴露）。
    // 复用上面 HTTP 同一份 identity_headers（HTTP/WS 单一真源，避免两处漂移）。
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
