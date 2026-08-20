# MessageV3 Phase1-4 与 Phase5 文字接龙真实 Gate 交接

## 2026-08-19 当前文字接龙交接（本节为最新真源）

```text
AGENT_HANDOFF/v1
state: REAL_CHAIN_TEXT_PASS
domain: Chain/Text
driver-native: 8 个 Phase5 Gate 已执行
business-evidence: 8 PASS / 0 BLOCKED_PRECONDITION
first-failure: none（本轮已修复并复验 CHAIN-COM-G03）
```

### 当前结论

9 个 canonical command 都已经进入 driver-native 的 Gate 映射并被真实 harness 调用；本轮并发
追加后的真实 Go WS 乱序帧已被保留并按 mutation 精确消费，8 个文字接龙相关业务 Gate 均取得
真实链 `PASS`。最新全量命令为：

```bash
HELIX_MAIN_API_BASE=http://api3-dev.jinqidongli.com \
  cargo test -p helix-driver-native --test message_v3_domain phase5_chain_text \
  -- --nocapture --test-threads=1
```

结果为 `8 passed; 0 failed`（1.84s）。Gate 业务账目必须读取同一轮生成的
`artifacts/messagev3-phase5-real-chain/**/evidence.json`：

| Gate | 最新业务状态 | 证据 |
| --- | --- | --- |
| CHAIN-COM-G01 | PASS | `hx-p1-evidence-81190-15-81190/.../CHAIN-COM-G01/evidence.json` |
| CHAIN-COM-G02 | PASS | `hx-p1-evidence-81190-26-81190/.../CHAIN-COM-G02/evidence.json` |
| CHAIN-COM-G03 | PASS | `hx-p1-evidence-81190-38-81190/.../CHAIN-COM-G03/evidence.json` |
| CHAIN-COM-G04 | PASS | `hx-p1-evidence-81190-53-81190/.../CHAIN-COM-G04/evidence.json` |
| CHAIN-COM-G05 | PASS | `hx-p1-evidence-81190-63-81190/.../CHAIN-COM-G05/evidence.json` |
| CHAIN-COM-G06 | PASS | `hx-p1-evidence-81190-78-81190/.../CHAIN-COM-G06/evidence.json` |
| CHAIN-TXT-G01 | PASS | `hx-p1-evidence-81190-88-81190/.../CHAIN-TXT-G01/evidence.json` |
| CHAIN-TXT-G02 | PASS | `hx-p1-evidence-81190-100-81190/.../CHAIN-TXT-G02/evidence.json` |

G03 本轮同时取得双 append 的真实 HTTP/WS、唯一 seq/entry、SQLite read-back 和 reconcile 幂等
证据；此前失败由 driver-native raw WS helper 丢弃乱序业务帧造成，不是 Go 持久化或 Helix
reducer 失败。

### 9 个 command 与 Gate 覆盖

| command | Gate 覆盖 |
| --- | --- |
| `post_chain_create` | COM-G01 及各 Gate setup |
| `post_chain_update_draft` | COM-G02 |
| `post_chain_publish` | COM-G02、COM-G03/04/05、TXT-G01/G02 setup |
| `post_chain_append` | COM-G03、COM-G05、TXT-G01、TXT-G02 |
| `post_chain_get` | COM-G01、COM-G04、TXT-G02 |
| `post_chain_reconcile` | COM-G03、TXT-G02 |
| `post_chain_close` | COM-G06 |
| `post_chain_retract` | COM-G06 |
| `post_chain_mark_read` | COM-G06 |

因此当前是“9 个 command 覆盖齐全，文字接龙相关真实 Gate 8/8”，但这不扩张为分类接龙或 Acceptance Gate 已通过。

### 本轮新增修复与验证

- `post_chain_mark_read` 的 HTTP authority 会复用已发布 Chain 摘要 revision；Helix 原先把它误
  当成重复 Chain 事实，revision 去重提前跳过 `PersistAtomic`，所以 G06 一直等不到
  `im:post_chain:read_cursor`。现已把 read-cursor 从 revision 去重条件排除，并保留 `req_id`
  到 terminal event；G06 已取得真实 `mark_read/retract/close/closed_append_rejected` PASS。
- G04 的 `post_chain_get` read relay 当前返回解包的 `WindowResult`，测试断言现兼容
  `/data/body/textWindow`，同时保留旧 CommonRes 路径回退，避免假失败。
- 修复 driver-native `RawWs` action matcher：不匹配当前等待器的带 action 业务帧进入有界
  `VecDeque`，后续等待按 action/predicate 消费，解决并发 append B→A 到达时 B 被丢弃的竞态。
- 新增 `read_cursor_event_preserves_req_id` Helix 单测；已通过：
  `cargo test -p helix-im chain::tests::read_cursor_event_preserves_req_id -- --nocapture`。

### 当前工作树边界

- Helix branch：`codex/messagev3-text-chain-helix-20260819`，HEAD `c3d6069465e4250609f05eb6012e42faf7cb5c3b`。
- 当前尚未提交的 Helix 修改：`crates/helix-im/src/chain.rs`、
  `crates/helix-im/src/port_reply/message_v3_chain.rs`、
  `crates/helix-driver-native/tests/messageV3-domain/shared/chain.rs`、
  `crates/helix-driver-native/tests/messageV3-domain/shared/mod.rs`。
- 本轮未 push、未把运行证据目录加入 Git；未改变 Go host `8066`，接龙验证仍使用
  `18066 -> container 8066`。
- Angular 生产组件/投影消费者未在本轮修改；因此尚无 Phase5 DOM/像素级验收证据。

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

该命令可完成编译并运行 8 个 Gate；Gate 测试仍按照项目合同将真实前置缺失写入
`BLOCKED_PRECONDITION`，因此最终账目必须读取同一轮
`artifacts/messagev3-phase5-real-chain/**/evidence.json`。本轮通过
`HELIX_MAIN_API_BASE=http://api3-dev.jinqidongli.com` 建立三账号同租户上下文，最新 run
`81190` 的 8 个业务 Gate 均为 `PASS`；host 端口仍为 `18066`，未占用 8066。

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

本轮最新 Phase5 真实证据（run `81190`）：

```text
CHAIN-COM-G02: PASS（draft_update、draft_fields、draft_idempotent_retry、stale_revision_rejected、published、raw_ws、sqlite_mutation）
CHAIN-COM-G03: PASS（concurrent_append、unique_seq、unique_entry、idempotent_reconcile、raw_ws、sqlite_mutation）
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
