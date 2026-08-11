# MessageV3 Phase1-4 真实 Gate 测试

## 目录合同

测试按 `PhaseN/Domain/Gate/main.rs` 拆分；`message_v3_domain.rs` 只负责注册固定的 84 个独立 Gate：Phase1=33、Phase2=20、Phase3=15、Phase4=16。共享模块通过 native driver 的 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 `run_engine_loop_with_transports` 接入真实 Go 服务，不创建 fake HTTP/WS，也不把 Node runner 或静态回放当作真实链。

84 个 Gate 都是默认执行的测试函数，不使用 `#[ignore]` 掩盖前置缺口。每个 Gate 都会启动真实链或明确探测真实前置；无法形成完整业务证据时，测试仍然结束并写入 `BLOCKED_PRECONDITION`、`NOT_VERIFIED` 或 `NOT_APPLICABLE`，不会把“测试函数返回 Ok”升级为业务 `PASS`。

## 运行

Go 服务必须已经运行在 `localhost:8066`；测试本身不要求用户配置环境变量，会自动读取 Angular 现有 `.wdio/credentials.local.json`。按 Phase 串行执行：

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
```

也可以执行完整 target：

```bash
cargo test -p helix-driver-native --test message_v3_domain -- --nocapture --test-threads=1
```

固定真实端点为 `http://127.0.0.1:8066/api/cses` 与 `ws://127.0.0.1:8066/api/v4/websocket`。health 只证明服务可达；一个业务 Gate 只有在自身合同要求的 HTTP admission、Go WS action、Helix projection、SQLite 持久行和 `im:__bus__` 证据都满足时才是 `PASS`。

## 证据

每个 Gate 写入最新一份：

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
```

证据不写入账号、cookie、密码、正文或完整 URL。普通发送链的 terminal 条件是合法 Go WS `post` 被 Helix 接收、出现 `im:post:received` 且 SQLite 行为 `send_status=sent`；失败链必须看到 `im:post:send-failed` 及对应 `failed/unsend` 状态。读、Pin、Bookmark、Announcement、Search、Todo 等 Gate 使用各自的 canonical event 与持久化读回条件。

### 当前真实运行账（84 Gate）

最近一次按 Phase1→Phase4 串行运行的证据汇总为：

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE |
| --- | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 20 | 1 | 11 | 1 |
| Phase2 | 20 | 9 | 7 | 0 | 4 |
| Phase3 | 15 | 15 | 0 | 0 | 0 |
| Phase4 | 16 | 6 | 10 | 0 | 0 |
| 合计 | 84 | 50 | 18 | 11 | 5 |

`cargo test` 的 `33/20/15/16 passed` 表示 84 个 harness 函数都执行并成功写出证据；上表才是业务 Gate 证据状态。当前主要阻塞是 Go media `media.type NOT NULL`、timeline producer/读回、Pin 的 `im:post:updated` 投影、Search/Todo 的 Angular/Helix 合同缺口，以及 Announcement 的异步帖子持久化读回。每一项都在对应 `evidence.json` 保留了 endpoint、req_id/correlation、事件和源码根因，不以 health/HTTP 200 代替终态。
