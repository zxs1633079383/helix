# MessageV3 Phase1-4 与 Phase5 文字接龙真实 Gate 测试

相关交付文档：

- [Phase1-4 非通过 Gate 证据缺口清单](Phase1-4-非通过Gate证据缺口清单.md)
- [真实 Gate handoff](handoff.md)

## 目录合同

测试按 `PhaseN/Domain/Gate/main.rs` 拆分；`message_v3_domain.rs` 注册固定 84 个既有 Gate 及 Phase5 文字接龙 8 个 Gate。共享 native harness 通过 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 engine loop 接入真实 Go，不创建 fake HTTP/WS。

Phase5 文字接龙入口固定为：

```text
Phase5/Chain/Common/CHAIN-COM-G01..G06/main.rs
Phase5/Chain/Text/CHAIN-TXT-G01..G02/main.rs
```

每个适用 Gate 都从公开 `Tick::Command` 进入，核对 18066 HTTP 业务信封、真实 `im:post_chain:*` WS frame、Helix `im:post_chain:*` EventSink、SQLite `chain_*` durable projection，并在独占频道/SQLite 上完成清理；纯查询 Gate 以 `im:read:result` 的 `req_id` 精确匹配代替 WS 回声。CHAIN-COM-G02 还覆盖草稿更新、同 mutation 幂等重试和 stale revision rejection；CHAIN-TXT-G02 覆盖 late-join GET 与 reconcile。分类接龙和跨域 `CHAIN-ACC-G01` 尚未纳入本批文字域入口。

每个 Gate 默认执行并写入自己的 `evidence.json`。真实前置不满足时，测试仍会明确写 `BLOCKED_PRECONDITION`、`NOT_VERIFIED` 或 `NOT_APPLICABLE`，不通过 `#[ignore]` 隐藏缺口。

## 运行

既有 Phase1-4 Go 服务仍使用 `localhost:8066`；Phase5 文字接龙使用 `localhost:18066`（Docker 容器内仍为 8066），测试自动读取 Angular 现有 credentials。登录服务默认沿用 release pre DNS；本地可用 `HELIX_MAIN_API_BASE` 覆盖登录服务地址，Go 接龙端口不变：

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase5_chain_text -- --nocapture --test-threads=1
```

完整 target：

```bash
cargo test -p helix-driver-native --test message_v3_domain -- --nocapture --test-threads=1
```

既有真实端点为 `http://127.0.0.1:8066/api/cses` 与 `ws://127.0.0.1:8066/api/v4/websocket`；Phase5 端点为 `http://127.0.0.1:18066/api/cses` 与 `ws://127.0.0.1:18066/api/v4/websocket`。health 只证明可达；业务 PASS 必须同时满足本 Gate 的 HTTP、Go WS、Helix projection、SQLite durable read-back 和 `im:__bus__` 合同。

## 当前证据账（2026-08-19）

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE |
| --- | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 22 | 5 | 5 | 1 |
| Phase2 | 20 | 16 | 0 | 0 | 4 |
| Phase3 | 15 | 15 | 0 | 0 | 0 |
| Phase4 | 16 | 16 | 0 | 0 | 0 |
| 合计 | 84 | **69** | **5** | **5** | **5** |

第一批 G03a、G11a/b/c/d/e/g/h、G07d 和第二批 G05f/g、G14a/b/c、G15a/b/c/d 已全部 PASS。剩余 5 个 BLOCKED 是 Angular Java/fileDrive/OSS 媒体字节平面，5 个 NOT_VERIFIED 是缺真实 producer/failed fixture 或本地 draft 合同，5 个 N/A 是本地生命周期/zero-write/派生状态边界；详见缺口清单。

Phase5 本轮聚焦证据为：`CHAIN-COM-G02` 草稿更新/幂等重试/发布 PASS，`CHAIN-TXT-G01` TEXT 字段与窗口 PASS，`CHAIN-TXT-G02` 双 session、late-join GET、reconcile 与双 SQLite PASS。Phase5 其余 Gate 仍按各自最新 `evidence.json` 单独计账，不并入上表的 Phase1-4 84 Gate 分母。

## 证据路径

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
artifacts/messagev3-phase5-real-chain/<run-id>/Phase5/Chain/{Common|Text}/<Gate>/evidence.json
```

按 Gate 取最新证据计算账面，不按历史运行目录数量相加。运行证据不入 Git，不保存账号、cookie、密码、DSN 或消息正文。
