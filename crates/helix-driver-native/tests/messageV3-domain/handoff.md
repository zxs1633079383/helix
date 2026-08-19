# MessageV3 Phase1-4 与 Phase5 文字接龙真实 Gate 交接

## 交接对象

- Helix worktree：`/System/Volumes/Data/workspace/worktrees/messagev3-text-chain-20260819/helix`
- Helix 分支：`codex/messagev3-text-chain-helix-20260819`
- Go 源码：`/System/Volumes/Data/workspace/worktrees/messagev3-text-chain-20260819/go-server`
- Angular 真源：`/System/Volumes/Data/workspace/angular/visual/cses-client`
- 既有 Phase1-4 Go HTTP/WS：`http://127.0.0.1:8066/api/cses` / `ws://127.0.0.1:8066/api/v4/websocket`
- Phase5 文字接龙 Go HTTP/WS：`http://127.0.0.1:18066/api/cses` / `ws://127.0.0.1:18066/api/v4/websocket`
- 推送状态：未 push；运行证据目录未加入 Git

## Phase5 文字接龙交付

新增并注册 8 个 Native Gate：

```text
crates/helix-driver-native/tests/messageV3-domain/Phase5/Chain/Common/CHAIN-COM-G01..G06/main.rs
crates/helix-driver-native/tests/messageV3-domain/Phase5/Chain/Text/CHAIN-TXT-G01..G02/main.rs
```

共享 harness 从公开 `Tick::Command` 进入 `helix-im`，由真实 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink` 驱动 18066。写 Gate 覆盖 Go HTTP CommonRes、`im:post_chain:*` raw WS、Helix projection、SQLite `chain_*` read-back 和 run-owned channel/SQLite 清理；查询 Gate 按 `req_id` 精确消费 `im:read:result`，避免把启动阶段其他读结果误认成接龙结果。G03 还检查双 append 的唯一 seq/entry 与 reconcile 幂等，TXT-G02 检查两个 Helix session 的同一 entry 和各自 SQLite。

本地 SQLite 表仍为 `chain_summary`、`chain_entry`、`chain_window`、`chain_mutation`、`chain_viewer` 及索引；本轮没有新增表，只为 `chain_summary` 增加 `title`、`description`、`deadline_at` 三个 additive 列及迁移，旧库可按已有重复列兼容策略升级。

Go 已补齐 `POST /post-chain/update-draft`：在 PostgreSQL `chain` 行锁内校验 creator、DRAFT 状态和 `expectedRevision`，原子更新摘要及公告快照，重复 mutation 幂等返回，草稿更新不广播公共频道事件。`sql/008_chain_text.up.sql` 已在本地真实 PostgreSQL 执行并核对五张表、摘要字段和索引；本地 SQLite 不需要新增表结构。

Docker 运行态复用此前 `make docker-up-clean` 拉起的容器：`cses-im-local-cses-im-server-1` 映射 `18066 -> 8066`，db/redis/pulsar healthy；host 8066 未被本轮占用。health 200 只记录可达，不作业务 Gate PASS。

## Phase5 验证边界

```bash
cargo test -p helix-driver-native --test message_v3_domain phase5_chain_text -- --nocapture --test-threads=1
```

该命令可完成编译并运行 8 个 Gate；Gate 测试按照项目合同将真实前置缺失写入 `BLOCKED_PRECONDITION`，因此 cargo 的 `8 passed` 只表示 harness 没有 panic，最终账目必须读取 `artifacts/messagev3-phase5-real-chain/**/evidence.json`。本轮曾捕获并修正 raw WS action matcher（Go 顶层 action 是 `im:post_chain:*`，不是 `post_chain`），但当前复验因登录服务 `cses-server-paas25-test.cses.svc.cluster.local:3399` DNS 不可解析，无法取得三账号同租户上下文；最新证据应按 `BLOCKED_PRECONDITION` 处理，不能宣称 8 个真实业务 Gate 已 PASS。

登录服务 DNS 不是 Go IM 服务提供：Angular 的 canonical pre 地址来自 `release.json` 的 `cses-server-paas25-test.cses.svc.cluster.local:3399`，本地测试可用 `HELIX_MAIN_API_BASE` 覆盖为可解析的 Java CSES 地址；Go 18066 只提供接龙 HTTP/WS。driver-native 默认仍使用 canonical pre DNS，覆盖变量只用于测试前置，不能把 DNS 不可解析归因于 Go 链路。

CHAIN-COM-G02 已完成草稿编辑、字段落库、同 mutation 幂等重试、旧 revision 冲突拒绝、publish revision=3、raw WS 和 SQLite mutation 断言；CHAIN-TXT-G02 已完成双 Helix session、同一 entry、late-join `post_chain_get`、`post_chain_reconcile`、actor/observer SQLite 对账。reconcile/draft 的 `eventSeq=0` 被明确视为非公共频道序号，不再误入 channel cursor gate。

## 本轮交付

本轮完成了用户指定的两批共 18 个 Gate：

1. 第一批：G03a、G11a/b/c/d/e/g/h、G07d，全部取得真实 PASS。
2. 第二批：G05f/g、G14a/b/c、G15a/b/c/d，全部取得真实 PASS。
3. 最新 84 Gate 账面为 `69 PASS / 5 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 5 NOT_APPLICABLE`，与目标一致。

## Helix 侧改动

- Phase2 Timeline 的 seed helper 从 Go response 提取真实 `createAt`，G11b/G11c 使用真实 authority anchor；G11d/G11h 适配同一 helper 返回值。
- Phase4 Announcement 的 seed request 补充 Go 所需 `teamId`，G07d 重新取得 announcement read/readers、SQLite 和 observer 证据。
- Phase4 Pin 共享 helper 增加置顶 projection 等待、SQLite `props.pinned` 轮询以及 server-id/temporary-id 双路径读回。
- G05g 先建立置顶为 true 的 setup projection，再验证 `im_set_message_untop` 的 Go remove route、WS、`im:post:updated` 和最终 false read-back。
- Search/Todo 的现有真实 probes 保留逐 Gate evidence；Todo 使用 Go 真实 `expedite` 类型，不用不存在的 `urgent` 假 fixture。
- `SESSION.md` 已记录本轮可证伪 EXPECT、DONE 和运行态偏差。

## Go 侧运行态修复

Pin 的直接真实请求曾返回：

```text
pq: there is no unique or exclusion constraint matching the ON CONFLICT specification
```

只读核对确认：Ent schema 声明了 `(channelId, postId)` unique，但当前运行数据库没有对应唯一约束；本轮没有直接执行生产 DDL。修复分两层：

- `internal/store/sqlstore/cses_post_pinned_store.go` 改为不指定 conflict target 的 `DO NOTHING`，让旧库不再因 PostgreSQL `42P10` 阻断现有 pin 链；具有正式唯一索引时仍由数据库保证幂等。
- 新增 `sql/006_post_pinned_unique.up.sql` 和 down migration，并在 `sql/README.md` 登记。up migration 带重复数据 preflight，需经过数据库变更审批后手工执行，不能由测试自动修改真实库。

Go 验证已通过：

```bash
go test ./internal/store/sqlstore -run 'TestCsesPost_Pinned$' -count=1
go test ./internal/app -run 'Test.*(Pinned|Pin)' -count=1
go build ./...
go vet ./...
```

## 证据状态

本轮最新 Phase5 真实证据：

```text
CHAIN-COM-G02: PASS（draft_update、draft_fields、draft_idempotent_retry、stale_revision_rejected、published、raw_ws、sqlite_mutation）
CHAIN-TXT-G01: PASS（mode=TEXT、no_category_ids、head_tail_window、sqlite_entry）
CHAIN-TXT-G02: PASS（two_helix_sessions、same_event_entry、late_join_get、reconcile、actor_sqlite、observer_sqlite）
```

对应证据位于 `artifacts/messagev3-phase5-real-chain/**/Phase5/Chain/Text/CHAIN-TXT-G0{1,2}/evidence.json`；必须以 `status` 字段为准，不能只看 cargo 的退出码。

按每个注册 Gate 选择最新 `artifacts/messagev3-phase1-4-real-chain/**/evidence.json` 后：

```text
Phase1: 22 PASS / 5 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 1 NOT_APPLICABLE
Phase2: 16 PASS / 0 BLOCKED_PRECONDITION / 0 NOT_VERIFIED / 4 NOT_APPLICABLE
Phase3: 15 PASS / 0 BLOCKED_PRECONDITION / 0 NOT_VERIFIED / 0 NOT_APPLICABLE
Phase4: 16 PASS / 0 BLOCKED_PRECONDITION / 0 NOT_VERIFIED / 0 NOT_APPLICABLE
总计:   69 PASS / 5 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 5 NOT_APPLICABLE
```

PASS 证据覆盖的共同终态是：`Tick::Command → Helix Persist → im:post:sending（适用时）→ Go HTTP → Go WS → Helix canonical event → SQLite read-back → NativeEventSink/im:__bus__`。读、Pin、Search、Todo、Announcement 各自还带对应 canonical event、correlation/req_id 和领域字段断言。

## 尚未完成的边界

- 5 个 BLOCKED 是 Angular Java `fileDrive`/OSS 字节平面未在同轮运行，不是 Go stable-reference 消费链失败。
- 5 个 NOT_VERIFIED 缺真实 TASK_CARD/VOTE/AVERAGE_SCORE/retry fixture 或按合同属于本地 draft owner。
- 5 个 NOT_APPLICABLE 是 zero-write、viewer/scroll、unread 派生和冷启动/重连生命周期；需要 native/Angular 宿主证据，不应强行增加 Go mutation。
- 本轮没有启动 WDIO，没有改 Angular 生产代码，没有直接执行数据库 migration，也没有 push。

## 继续复验

```bash
cargo test -p helix-driver-native --test message_v3_domain phase1_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase2_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase3_ -- --nocapture --test-threads=1
cargo test -p helix-driver-native --test message_v3_domain phase4_ -- --nocapture --test-threads=1
```

这些命令需要 Go 已运行在 `localhost:8066`；不需要用户配置环境变量。若只验证本轮目标，使用缺口清单中的 focused 命令并重新检查对应最新证据即可。
