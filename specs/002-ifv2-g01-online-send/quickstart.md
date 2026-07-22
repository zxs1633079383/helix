# Validation Quickstart: SPEC-IFV2-G01-在线发送

## Preconditions

1. 先只读核验 `specs/001-gs-sync-recovery/contracts/sync-recovery-evidence.md` 的 T050：它必须对**当前 source pin**明确记录 Rust、PC DOM、mobile fixture、SQLite/PG readonly 四面 `PASS`。未满足时结论是 `BLOCKED`，仅可更新本 Spec 文档/审计记录；不得创建或使用 G01 实现 worktree、运行 Cargo/Jest/真实链，或修改 source/fixture。
2. 将经 T050 核验的 source pin、四面证据路径、核验时间和 G01 `corrKey` evidence-packet layout 记录在本文件的本次运行记录中。设计文档、历史 pin、单端 fixture 或“001 正在跑”都不是准入证据。
3. 使用 admitted implementation pin、可清理频道及隔离账号 A（sender）、B（receiver）。
4. 仅启动正常产品链，不手改 SQLite、PostgreSQL、Frame Cache 或 DOM。
5. 固定一个 `corrKey` / `requestId`，打开 action ingress、Tick、Effect、port reply、SurfaceComposer、frame apply、FrameApplied、directive 的结构化 hop 日志。
6. A 必须从 001 hydration completed 的 `message.channel-summary + message.composer + runtime` bundle 获得当前 `message.create` capability；binding 的 fixed action kind 是 `im.send-message`，客户端 invocation 只提交 `actionRef` 与 `im.message-content.v2` 输入。陌生频道先完成 GS01-GS04 bundle。

## Required test commands

只有 T001 准入成功、且相应实现任务完成后，才运行以下 focused Rust tests，再运行由工作区决定的更宽测试：

```bash
cargo test -p helix-im --test send_test
cargo test -p helix-im --test f1_send_http_reconcile_test
cargo test -p helix-im --test interaction_frame_contract_test
cargo test -p helix-im --test f5_golden_replay
```

在 `/System/Volumes/Data/workspace/angular/cses-client`，先运行 implementation task 记录的 message-v4 focused Jest/Angular DOM 命令，再运行当前 package script 的 CI-equivalent。报告每条命令开始、结束、耗时、cache/retry/skip；不得猜造 package script 名称。

## Scenario A: accepted text to Sent

1. A 用从 001 bundle 获得的有效 G01 binding 提交 text。
2. 验证 accepted receipt 与 `PersistAtomic(temporary message + pending send)` Effect。
3. 验证 `PersistOk` 生成一份 `Sending` V2 frame；cache 原子 apply，且 request-scoped directive 在 `FrameApplied` 后清 draft。
4. 验证 HTTP 2xx 后 row 仍是 `Sending`。
5. 投递 matching authoritative echo/sync；验证 canonical write `PersistOk` 与同一逻辑 `Sent` row/server id。
6. 验证 B 在自己的 canonical inbound/sync 前看不到 sender temporary，之后只看 canonical message。

通过需要 [data-model.md](./data-model.md) 的完整 evidence packet，且没有重复逻辑 row。

## Scenario B: failure counterexamples

分别留存 fixture/log：

- stale/invalid binding 或 invalid text：rejected receipt、恢复原 draft、无 row；
- temporary persist error：恢复 draft、无 Sending row；
- remote timeout/retryable 与 nonretryable error：persist 后同一 row `Failed`，仅合法 retry binding 出现；
- HTTP 2xx before echo：仍为 Sending；
- duplicate/late echo：仅一条 canonical Sent row；
- corrupt/unknown/revision-gap frame：cache 完全不变并请求 resync。

## Read-only fact reconciliation

用 narrow tenant/user/channel/correlation predicate 调 MCP，只记录 query、时间和返回标识（不记录 secrets）：

| Plane | Read-only evidence |
|---|---|
| Local SQLite | `message`/pending correlation、channel state、可用 local application ledger；物理表缺失是 block，不能伪造。 |
| Remote PG | isolated run 关联的 canonical `post`、`channel_event`、`channel_member`、`post_idempotency` / delivery record。 |
| Sender DOM | 仅一个逻辑 row，`Sending` -> `Sent` 或正确 `Failed`；draft clear/recovery 与 causation 对齐。 |
| Receiver DOM | 无 sender temporary；仅在 B own persisted inbound/sync 后见 canonical message/preview/unread。 |

`mcp__helix_local_sqlite_ro` 与 `mcp__cses_pre_postgres_ro` 是 bounded SELECT-only。查询缺行、DSN 不可用、关联不能建立时结果为 `blocked`/`inconclusive`。

## Human intervention and reporting

raw emit 到达 V2 UI、frame validation 失败、live request 的 actionRef 反复过期、persist/remote record 分叉、receiver 看见 Sending、必需 DB/DOM 面不可用时，停止并人工介入。授权工程师可修 source/test 后重跑；禁止 direct data write、cache edit、fixture-only UI fabrication。

工作超过 30 分钟时每 30 分钟报告完成项、证据、阻塞和下一步。所有 test/real-chain run 还要附 command start/end/duration。
