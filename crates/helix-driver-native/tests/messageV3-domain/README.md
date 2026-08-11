# MessageV3 Phase1-4 真实 Gate 测试

## 目录合同

测试按 `PhaseN/Domain/Gate/main.rs` 拆分；`message_v3_domain.rs` 只负责注册 84 个独立 Gate。共享模块通过 native driver 的 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 `run_engine_loop_with_transports` 接入真实 Go 服务，不创建 fake HTTP/WS。

Phase1 已具备真实实现的 Gate 默认执行；媒体、retry、draft、本地大文件负向以及 Phase2-4 在真实前置未满足时使用 `#[ignore]`，运行 ignored 只会写 `NOT_VERIFIED` 或 `NOT_APPLICABLE` 证据，不会写 PASS。

## 运行

```bash
cargo test -p helix-driver-native \
  --test message_v3_domain \
  -- --nocapture --test-threads=1
```

命令不需要配置环境变量。测试固定连接 `http://127.0.0.1:8066/api/cses` 和 `ws://127.0.0.1:8066/api/v4/websocket`，并自动读取 Angular 现有 `.wdio/credentials.local.json`。health 只作为前置，业务通过必须同时看到 HTTP `status=SUCCESS`、WS action、Helix projection、SQLite 行状态和 `im:__bus__`。

## 证据

每个完成的 Gate 写入：

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
```

证据不写入账号、cookie、密码、正文或完整 URL。`send` 链的 terminal 条件是 `im:post:received` 且 SQLite `send_status=sent`；失败链必须看到 `im:post:send-failed` 及 `unsend/failed` 行状态。

本轮 localhost:8066 实测：Phase1 `19 PASS / 13 NOT_VERIFIED / 1 NOT_APPLICABLE`；Phase2 `20 NOT_VERIFIED`、Phase3 `15 NOT_VERIFIED`、Phase4 `16 NOT_VERIFIED`。这不是把后续 Phase 提前放行：只有上一 Phase 的 84 Gate 前置全部达到合同终态，才允许把对应 `#[ignore]` 改为真实执行。
