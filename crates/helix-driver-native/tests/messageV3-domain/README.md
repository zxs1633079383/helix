# MessageV3 Phase1-5 真实 Gate 测试

相关交付文档：

- [Phase1-4 非通过 Gate 证据缺口清单](Phase1-4-非通过Gate证据缺口清单.md)
- [真实 Gate handoff](./handoff.md)

## 目录合同

测试按 `PhaseN/Domain/Gate/main.rs` 拆分；统一 runner 从目录与注册表发现并校验 Phase1-5 共 83 个可执行 Gate：32 + 16 + 15 + 12 + 8。客户端目录仍保留 92 Gate 的完整合同，但 `MV3-G05L`、`MV3-G06C`、`MV3-G06E`、`MV3-G11F`、`MV3-G13E` 以及 `CHAIN-CAT-G01..G03`、`CHAIN-ACC-G01` 不属于 real-chain 执行分母。Phase4/Todo 下的历史 G15a-d 也不属于当前分母。共享 native harness 通过 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 和 engine loop 接入真实 Go，不创建 fake HTTP/WS。

Phase5 文字接龙入口固定为：

```text
Phase5/Chain/Common/CHAIN-COM-G01..G06/main.rs
Phase5/Chain/Text/CHAIN-TXT-G01..G02/main.rs
```

每个可执行 Gate 都从公开 `Tick::Command` 进入，按合同核对真实 Go HTTP/WS、Helix projection、SQLite durable projection、NativeEventSink 和 run-owned 清理；纯查询 Gate 以 `im:read:result` 的 `req_id` 精确匹配代替 WS 回声。CHAIN-COM-G02 还覆盖草稿更新、同 mutation 幂等重试和 stale revision rejection；CHAIN-TXT-G02 覆盖 late-join GET 与 reconcile。Category/Acceptance 目录保留为生产能力记录，但不再由 real-chain runner 选择或执行。

每个 Gate 默认执行并写入自己的 `evidence.json`。真实前置不满足时，测试仍会明确写 `BLOCKED_PRECONDITION`、`NOT_VERIFIED` 或 `NOT_APPLICABLE`，不通过 `#[ignore]` 隐藏缺口。

## 运行

所有 Phase1-5 真实链使用 `scripts/messagev3/real-chain-environment.mjs` 的环境 profile；默认 `pre`，本地可用 `MESSAGEV3_ENV=local`，未来环境可通过同名端点变量扩展。测试优先读取 GitLab File 变量 `CSES_WDIO_CREDENTIALS_FILE`，本地再回退到 Angular 现有 credentials：

```bash
# 列出 83 个可执行 Gate
node scripts/messagev3/phase1-5-real-chain.mjs --list
# 按 Phase / Domain / Gate 精确执行
node scripts/messagev3/phase1-5-real-chain.mjs --phase 1
node scripts/messagev3/phase1-5-real-chain.mjs --domain channel_member
# 切换本地 profile
MESSAGEV3_ENV=local node scripts/messagev3/phase1-5-real-chain.mjs --all
# 全量 83 Gate（任一非 PASS 状态均非零退出）
node scripts/messagev3/phase1-5-real-chain.mjs --all
```

需要绕过 runner 进行单个 Rust Gate 调试时，可显式选择 profile：

```bash
MESSAGEV3_ENV=local cargo test -p helix-driver-native --test message_v3_domain phase1_post_g01a -- --nocapture --test-threads=1
```

默认 pre 端点为 `http://api-im-pre.jinqidongli.com/api/cses`、`ws://ws-im-pre.jinqidongli.com/api/v4/websocket` 与 `https://api-pre.jinqidongli.com`；local profile 保留原 `127.0.0.1:18066`/`127.0.0.1:3399`。也可分别设置 `MESSAGEV3_GO_HTTP_BASE`、`MESSAGEV3_GO_WS_URL`、`MESSAGEV3_JAVA_API_BASE`。health 只证明可达；业务 PASS 必须同时满足本 Gate 的 HTTP、Go WS（适用时）、Helix projection、SQLite durable read-back（适用时）和 NativeEventSink 合同。

## 变更前历史证据账（2026-08-31，过滤前 92 Gate）

| Phase | Gate 数 | PASS | BLOCKED_PRECONDITION | NOT_VERIFIED | NOT_APPLICABLE | NOT_IMPLEMENTED |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Phase1 | 33 | 23 | 5 | 4 | 1 | 0 |
| Phase2 | 20 | 16 | 0 | 0 | 4 | 0 |
| Phase3 | 15 | 15 | 0 | 0 | 0 | 0 |
| Phase4 | 12 | 12 | 0 | 0 | 0 | 0 |
| Phase5 | 12 | 8 | 0 | 0 | 0 | 4 |
| 合计 | **92** | **74** | **5** | **4** | **5** | **4** |

这张表是执行过滤改动前的历史账，不代表新的 83 Gate 分母。历史非 PASS 是可证伪阻塞，不是 Cargo 失败：Phase1 的 G01h/i/j/G02c 缺批准的真实 producer/failed fixture，G01c/d/e/g/G03c 缺 Angular→Java fileDrive/OSS 媒体字节平面，G05l 是合同规定的本地 zero-write Gate；Phase5 Category/Acceptance 在当前生产实现中仅支持 TEXT，因此明确为 `NOT_IMPLEMENTED`。Phase4 Search 已使用用户提供的 ES 只读索引 fixture 真连恢复：通过 Kubernetes `es940-es-http` 的临时 port-forward 与临时 Go 18066 运行态，最新完整 Phase4 重验 12/12 PASS（Search-only r4 为 3/3 PASS）；临时授权、配置、容器和 port-forward 均未写入仓库、证据或日志，测试后已恢复原始 Go 容器。健康检查、HTTP 2xx、静态目录和单层 WS 均未升级为 PASS。

最新历史 suite 目录（过滤前）：Phase1 `final-20260831-p1-r4/messagev3-1788178972331-11792`，Phase2 `final-20260831-p2-r2/messagev3-1788178653178-8921`，Phase3 `final-20260831-p3-r3/messagev3-1788178661456-9034`，Phase4 `final-20260831-p4-r5/messagev3-1788180031564-23636`，Phase5 `final-20260831-p5-r2/messagev3-1788178620418-8622`；Phase4 Search-only 重验为 `final-20260831-p4-r4/messagev3-1788179819969-21362`。Phase4 G14a 当前同一 Native 会话依次断言 `im_search_do`、`im_search_user`、`im_search_channel` 三条真实 Go 搜索链；Phase5 Common/Text 8 Gate 通过真实 Go/WS/SQLite/EventSink，Category/Acceptance 的 fail-closed 证据也纳入同一 92 Gate 分母。Docker 重建后的 Phase4 r3 曾受 ES fake-IP 隧道不可达影响，后续 port-forward r4/r5 已恢复 Search 三 Gate 及完整 Phase4；新的 83 Gate runner 尚未产生历史 suite。

## 证据路径

```text
artifacts/messagev3-phase1-5-real-chain/<suite-run-id>/<Phase>/<canonical-domain>/<GATE>/evidence.json
artifacts/messagev3-phase1-5-real-chain/<suite-run-id>/summary.json
```

按 suite 取最新证据计算账面，不按历史运行目录数量相加。运行证据不入 Git，不保存账号、cookie、密码、DSN 或消息正文；证据字段只保留必要的业务 ID、状态和逐跳结果。

变更前的 92/92 仅是 Gate 测试类与历史分母覆盖；当前环境未安装 `cargo-llvm-cov`，不把 Rust 源码行覆盖率冒充为 100%。
