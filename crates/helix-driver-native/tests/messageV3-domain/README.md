# MessageV3 Phase1-4 真实 Gate 测试

相关交付文档：

- [Phase1-4 非通过 Gate 证据缺口清单](Phase1-4-非通过Gate证据缺口清单.md)
- [真实 Gate handoff](handoff.md)

## 目录合同

测试按 `PhaseN/Domain/Gate/main.rs` 拆分；`message_v3_domain.rs` 注册固定 84 个独立 Gate：Phase1=33、Phase2=20、Phase3=15、Phase4=16。共享 native harness 通过 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 engine loop 接入真实 Go，不创建 fake HTTP/WS。

每个 Gate 默认执行并写入自己的 `evidence.json`。真实前置不满足时，测试仍会明确写 `BLOCKED_PRECONDITION`、`NOT_VERIFIED` 或 `NOT_APPLICABLE`，不通过 `#[ignore]` 隐藏缺口。

## 运行

Go 服务必须运行在 `localhost:8066`；测试自动读取 Angular 现有 credentials，不要求用户配置环境变量：

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
```

完整 target：

```bash
cargo test -p helix-driver-native --test message_v3_domain -- --nocapture --test-threads=1
```

真实端点为 `http://127.0.0.1:8066/api/cses` 与 `ws://127.0.0.1:8066/api/v4/websocket`。health 只证明可达；业务 PASS 必须同时满足本 Gate 的 HTTP、Go WS、Helix projection、SQLite durable read-back 和 `im:__bus__` 合同。

## 当前证据账（2026-08-11）

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE |
| --- | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 22 | 5 | 5 | 1 |
| Phase2 | 20 | 16 | 0 | 0 | 4 |
| Phase3 | 15 | 15 | 0 | 0 | 0 |
| Phase4 | 16 | 16 | 0 | 0 | 0 |
| 合计 | 84 | **69** | **5** | **5** | **5** |

第一批 G03a、G11a/b/c/d/e/g/h、G07d 和第二批 G05f/g、G14a/b/c、G15a/b/c/d 已全部 PASS。剩余 5 个 BLOCKED 是 Angular Java/fileDrive/OSS 媒体字节平面，5 个 NOT_VERIFIED 是缺真实 producer/failed fixture 或本地 draft 合同，5 个 N/A 是本地生命周期/zero-write/派生状态边界；详见缺口清单。

## 证据路径

```text
artifacts/messagev3-phase1-4-real-chain/<run-id>/<Phase>/<Domain>/<Gate>/evidence.json
```

按 Gate 取最新证据计算账面，不按历史运行目录数量相加。运行证据不入 Git，不保存账号、cookie、密码、DSN 或消息正文。
