---

description: "Spec 047 MessageV3 Gate 事件重构任务清单"
---

# Tasks: IM Event Gate 清单与 MessageV3 硬切

**Input**: `spec.md`、`plan.md`、`research.md`、`data-model.md`、`contracts/`、`quickstart.md`

**Scope**: 仅 Helix 仓内 30 个 Gate。当前版本生成 `.v1` JSON，不同步
`cses-client` WDIO 或 Mobile GTest。

**执行纪律**: 严格按 P0→P7 串行。每个 Gate 先写会失败的真实
`ImModule::handle` 合同测试，再实现 MessageV3，最后删除该 Gate 的 V2/V1 路径。迁移中的
Gate 不允许双写，也不允许借用其他证据面宣称 Green。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 仅表示文件无冲突且无前置依赖；本清单的 Gate 硬切不并行。
- **[US1]**: 30 Gate 分母与证据分级。
- **[US2]**: post、schedule、read 事件。
- **[US3]**: channel、member、topic 事件。
- **[US4]**: timeline、sync、recovery 事件。
- **[US5]**: 生成证据、清除 V2/V1、收口治理。

## Phase 1: P0 治理与迁移边界

**Goal**: 先让 Constitution、ADR、ROADMAP、项目入口和机器清单共同承认
`Effect::Emit { event, data }` 是唯一 MessageV3 真源。

- [X] T001 [US5] 对账 Constitution 2.0.0，并在 `docs/GOAL.md` 删除 InteractionFrame V2、Surface、FrameApplied 作为目标合同的表述
- [X] T002 [US5] 在 `docs/harness/decisions.md` 新增取代 ADR-015 的 MessageV3 决策并把 ADR-015 标为 superseded，保留历史而不伪装为现状
- [X] T003 [US5] 更新 `docs/ROADMAP.md`，把 P0→P7、30 Gate 硬切、Helix-only 产物与跨仓同步延期写入活账
- [X] T004 [US5] 更新 `AGENTS.md` 的行为真源、多端 checklist 与文档索引，移除 Frame/Surface 作为当前目标的指令
- [X] T005 [US1] 在任何生产编码前创建 `specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json` manifest，登记精确 30 Gate、阶段、`domain`（FR-005 七域取值）、`test_package`、`test_path`、显式测试目标、生产 owner 与完整 V2/V1/客户端/文档删除清单；现存引用标 `PENDING_REMOVAL`，`channel-member` 映射到 `channel_member`，G-13/G-14 记为 `read`，未知项不得用空数组或 `TODO` 逃逸
- [X] T006 [US5] 新增 `specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs`，校验 Gate 集合、domain/package 映射、迁移状态、禁用引用和跨仓零写入，并合并 `review-ledger.json` 的 P0/P7 与 migration manifest 的 Gate 状态，断言 `IN_PROGRESS` 条目 ≤1
- [X] T007 [US5] 更新 `specs/047-im-event-gate-inventory/verification.md`，拆分 deterministic、contract、integration、runtime、dual-user、canonical 六级证据
- [X] T008 [US5] 更新 `specs/047-im-event-gate-inventory/discovery.md` 与 `specs/047-im-event-gate-inventory/review-ledger.json`，记录本轮 30 Gate 分母和 G-03/G-17..G-27 排除边界

**Checkpoint**: 治理文件、迁移 manifest 和验证口径一致；不得开始 Gate 编码前保留
“InteractionFrame V2 是唯一真源”的有效规范。

## Phase 2: P0 MessageV3 公共基础

**Goal**: 建立无 Frame/Surface 的事件模块、持久化后释放规则和唯一 Rust 产物生成链。

### Tests First

- [X] T009 [US5] 新增 `crates/helix-im/tests/message_v3/contracts/event/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_event_contract`，先断言事件只含 `event` 与 `data`、禁止 Frame/Surface 字段，并以失败用例证明 matching `PersistOk` 前无 terminal Emit、匹配后恰好释放一次
- [X] T010 [US5] 新增 `crates/helix-im/tests/message_v3/contracts/artifact_schema/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_artifact_schema`，先断言四类 generated JSON 满足 `contracts/*.schema.json` 并观察失败
- [X] T011 [US1] 新增 `crates/helix-im/tests/message_v3/contracts/gate_registry/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_gate_registry`，先断言 manifest 与显式 test target 精确覆盖 30 Gate、无重复、无 G-03/G-17..G-27

### Implementation

- [X] T012 [US5] 新增 `crates/helix-im/src/event/mod.rs` 与 `crates/helix-im/src/event/{post,channel,channel_member,schedule,timeline,read,sync}.rs`，提供按七个业务域构造 `Effect::Emit` 的类型化入口
- [X] T013 [US5] 新增 `crates/helix-im/tests/message_v3/support/{artifact,scenario}.rs`，从真实 Tick、`ImModule::handle`、完整 Effect、持久化读回和 persona 输出确定性 JSON
- [X] T014 [US5] 在 `crates/helix-im/src/module/` 与 `crates/helix-im/src/port_reply/` 建立 PersistOk 后才释放 terminal MessageV3 事件的公共编排
- [X] T015 [US5] 重写 `specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs`，只编排 Rust 测试、schema 校验、索引和 hash，不再拼装业务 payload
- [X] T016 [US5] 在 `crates/helix-im/tests/message_v3/contracts/artifact_schema/main.rs` 增加相同输入重复生成 byte/hash 稳定性和敏感字段拒绝测试
- [X] T017 [US5] 对账并冻结 `specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json` 的预编码删除清单，补入公共基础开发中新发现的共享引用，证明 30 Gate 均无未知 owner、空清单或 `TODO`
- [X] T018 [US5] 运行 P0 聚焦验证并把命令、失败分类和当前证据等级写回 `specs/047-im-event-gate-inventory/verification.md`

**Checkpoint**: 公共基础 Green，但尚未迁移的 Gate 仍可临时引用共享旧基础；任何已迁移
Gate 从此只能走 MessageV3。

## Phase 3: User Story 1 - 精确 Gate 分母 (Priority: P1)

**Goal**: 让测试、manifest、spec 和生成目录都严格等于用户确认的 30 Gate。

**Independent Test**: 运行 registry test，缺一个、重复一个或混入 G-03/G-17..G-27 均失败。

- [X] T019 [US1] 在 `crates/helix-im/tests/message_v3/contracts/gate_registry/main.rs` 对账 Gate **集合**（spec 30 Gate ↔ manifest ↔ 目录 ↔ `Cargo.toml` 目标，missing/extra/duplicate=0）与每 Gate **domain/package 映射**（同名映射，唯 `channel-member -> channel_member`）；spec §A-E 审核分组不参与集合/域断言，仅 domain 字段为权威
- [X] T020 [US1] 在 `specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs` 校验 `.v1/gates/*/generated` 目录集合只能是精确 30 Gate
- [X] T021 [US1] 更新 `specs/047-im-event-gate-inventory/verification.md`，把已有测试源、WDIO 场景源、缺失 receiver 断言和 INFERRED 行为分别列明

**Checkpoint**: 分母可由机器证明，证据等级不互借。

## Phase 4: P1 基线 Gate - G-15a 与 G-01

**Goal**: 先用建群建立 channel/member 基线，再用发文本消息建立 post、lastPost 和 unread 基线。

### G-15a 创建群聊

- [X] T022 [US3] 新增 `crates/helix-im/tests/message_v3/channel/g15a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g15a`，先覆盖 owner/invitee 入站、持久化、summary、roster、role 与双视角 Emit 并观察失败
- [X] T023 [US3] 在 `crates/helix-im/src/event/channel.rs`、`crates/helix-im/src/port_reply/dispatch_channel_create.rs`、`crates/helix-im/src/ws/handlers/channel_created.rs` 实现 G-15a，生成四类 JSON，删除该 Gate 的 Frame/Surface/V1 分支并更新迁移 ledger

### G-01 发送文本消息

- [X] T024 [US2] 新增 `crates/helix-im/tests/message_v3/post/g01/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g01`，先覆盖 sender/receiver post、channel lastPost、sender unread=0、receiver unread+1、PersistOk 后 Emit 并观察失败
- [X] T025 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/port_reply/`、`crates/helix-im/src/ws/handlers/post.rs`、`crates/helix-im/src/channel/write/post.rs` 实现 G-01，生成四类 JSON，删除该 Gate 旧路径并更新迁移 ledger

**Checkpoint**: G-15a、G-01 已硬切；两者不得再产出 Frame/Surface 或旧 V1 事件。

## Phase 5: P2 Post 行为 Gate

**Goal**: 以同一 post identity 和 persona 独立状态迁移消息行为。

- [ ] T026 [US2] 新增 `crates/helix-im/tests/message_v3/post/g02/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g02`，先覆盖文档消息 sender/receiver、附件数据和 channel preview 并观察失败
- [ ] T027 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/ws/handlers/post.rs` 实现 G-02，生成 JSON，删除该 Gate 旧投影并更新 ledger
- [ ] T028 [US2] 新增 `crates/helix-im/tests/message_v3/post/g03a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g03a`，先覆盖初次失败仅 sender、retry 复用 temporary_id、成功后 receiver 才可见并观察失败
- [ ] T029 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/port_reply/dispatch.rs` 实现 G-03a retry，生成 JSON，删除旧 sending/sent/failed fallback 并更新 ledger
- [ ] T030 [US2] 新增 `crates/helix-im/tests/message_v3/post/g04/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g04`，先覆盖 `im:post:revoke`、同 post 幂等更新、双视角系统提示和 lastPost 更新并观察失败
- [ ] T031 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/ws/handlers/posts_update.rs`、`crates/helix-im/src/ws/handlers/update_channel.rs` 实现 G-04，生成 JSON，删除旧 revoke Frame/alias 并更新 ledger
- [ ] T032 [US2] 新增 `crates/helix-im/tests/message_v3/post/g04a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g04a`，先覆盖离线同步前状态、撤回 replay 幂等与冷启动读回并观察失败
- [ ] T033 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/sync/flow.rs` 实现 G-04a，生成 JSON，删除旧 replay 投影并更新 ledger
- [ ] T034 [US2] 新增 `crates/helix-im/tests/message_v3/post/g05/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g05`，先覆盖源消息不变、目标频道新 post、目标 receiver 与 dialog preview 并观察失败
- [ ] T035 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/port_reply/dispatch.rs` 实现 G-05，生成 JSON，删除旧 forward Frame/VM 并更新 ledger
- [ ] T036 [US2] 新增 `crates/helix-im/tests/message_v3/post/g06/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g06`，先覆盖 quick_reply JSONB 幂等、sender/receiver reaction 终态与冷启动读回并观察失败
- [ ] T037 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/ws/handlers/post_update.rs` 实现 G-06，生成 JSON，删除旧 reaction 投影并更新 ledger
- [ ] T038 [US2] 新增 `crates/helix-im/tests/message_v3/post/g07/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g07`，先覆盖加急创建、确认完成、参与人独立 pending/completed 与 roster 并观察失败
- [ ] T039 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/ws/handlers/post_update.rs` 实现 G-07，生成 JSON，删除旧 urgent Frame/VM 并更新 ledger
- [ ] T040 [US2] 新增 `crates/helix-im/tests/message_v3/read/g14/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g14`，先覆盖单条已读、发送者回执视图、参与人独立状态与恢复并观察失败
- [ ] T041 [US2] 在 `crates/helix-im/src/event/read.rs`、`crates/helix-im/src/ws/handlers/post_read.rs`、`crates/helix-im/src/port_reply/dispatch_read.rs` 实现 G-14，生成 JSON，删除旧 read Frame 并更新 ledger
- [ ] T042 [US2] 新增 `crates/helix-im/tests/message_v3/post/g14a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g14a`，先覆盖模板确认、权威已收到状态与冷启动恢复并观察失败
- [ ] T043 [US2] 在 `crates/helix-im/src/event/post.rs`、`crates/helix-im/src/ws/handlers/post_update.rs` 实现 G-14a，生成 JSON，删除旧 template Frame/VM 并更新 ledger

**Checkpoint**: 九个 Post 行为 Gate 全部以同一 identity 幂等更新，未完成的 receiver 行为保持
MISSING/INFERRED，不用手写 JSON 填绿。

## Phase 6: P3 Schedule Gate

**Goal**: 定时消息创建/取消保持 creator 私有，实际触发后的普通消息由 G-01 合同负责。

- [ ] T044 [US2] 新增 `crates/helix-im/tests/message_v3/schedule/g08/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g08`，先覆盖空 channel summary、定时状态、creator-only Emit 与冷启动恢复并观察失败
- [ ] T045 [US2] 在 `crates/helix-im/src/event/schedule.rs`、`crates/helix-im/src/ws/handlers/post_schedule_created.rs` 实现 G-08，生成 JSON，删除旧 schedule Frame 并更新 ledger
- [ ] T046 [US2] 新增 `crates/helix-im/tests/message_v3/schedule/g09/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g09`，先覆盖已定时到取消终态、creator-only Emit 与冷启动恢复并观察失败
- [ ] T047 [US2] 在 `crates/helix-im/src/event/schedule.rs`、`crates/helix-im/src/ws/handlers/post_schedule_canceled.rs` 实现 G-09，生成 JSON，删除旧 schedule Frame 并更新 ledger

**Checkpoint**: G-08/G-09 不制造 receiver 消息；触发投递仍复用 G-01。

## Phase 7: P4 Timeline 与 Read Gate

**Goal**: timeline window、anchor、token、append 与已读状态都按 persona 独立持久化和释放。

- [ ] T048 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g10/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g10`，先覆盖 timeline ready、缓存降级、HTTP 缺失/超时和恢复并观察失败
- [ ] T049 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/query/render_ready/` 实现 G-10，生成 JSON，删除旧 ready Frame 并更新 ledger
- [ ] T050 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g10a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g10a`，先覆盖 older page 合并、历史耗尽、HTTP/持久化失败、竞态与 generation 并观察失败
- [ ] T051 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/channel/older_context/` 实现 G-10a，生成 JSON，删除旧 pagination Frame 并更新 ledger
- [ ] T052 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g10b/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g10b`，先覆盖 revision-1 snapshot、Delta 定位、newer 尾插与耗尽边界并观察失败
- [ ] T053 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/query/render_ready/` 实现 G-10b，生成 JSON，删除旧 newer Frame 并更新 ledger
- [ ] T054 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g10c/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g10c`，先覆盖满窗口 append、头部淘汰和 consumer 本地选择器职责并观察失败
- [ ] T055 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/channel/write/post.rs` 实现 G-10c，生成 JSON，删除旧 append Frame 并更新 ledger
- [ ] T056 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g10d/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g10d`，先覆盖离底 anchor 固定、新消息提示、ActivateWindow 副作用边界并观察失败
- [ ] T057 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/query/render_ready/` 实现 G-10d，生成 JSON，删除旧 anchor Frame 并更新 ledger
- [ ] T058 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g11/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g11`，先覆盖初始 bounded window、定位目标后的 active window 与滚动指令数据并观察失败
- [ ] T059 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/query/render_ready/` 实现 G-11，生成 JSON，删除旧 locate Frame 并更新 ledger
- [ ] T060 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g11a/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g11a`，先覆盖未缓存定位、双向分页乱序、当前 token 激活与旧响应只合并数据并观察失败
- [ ] T061 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/channel/older_context/` 实现 G-11a，生成 JSON，删除旧 locate Frame 并更新 ledger
- [ ] T062 [US4] 新增 `crates/helix-im/tests/message_v3/timeline/g12/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g12`，先覆盖 thread snapshot、回复列表与话题分支数据并观察失败
- [ ] T063 [US4] 在 `crates/helix-im/src/event/timeline.rs`、`crates/helix-im/src/query/render_ready/` 实现 G-12，生成 JSON，删除旧 thread Frame 并更新 ledger
- [ ] T064 [US2] 新增 `crates/helix-im/tests/message_v3/read/g13/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g13`，先覆盖 viewer 会话已读、其他成员未读不变与冷启动恢复并观察失败
- [ ] T065 [US2] 在 `crates/helix-im/src/event/read.rs`、`crates/helix-im/src/port_reply/dispatch_read.rs` 实现 G-13，生成 JSON，删除旧 dialog-read Frame 并更新 ledger

**Checkpoint**: timeline/read 九个 Gate 的窗口和状态是 persona-local，UI 导航指令不再套在
Frame/Surface 中。

## Phase 8: P5 Channel、Member 与 Topic Gate

**Goal**: 频道生命周期和成员状态向有权观察者收敛，角色与本地导航仍按 viewer 独立。

- [ ] T066 [US3] 新增 `crates/helix-im/tests/message_v3/channel/g15/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g15`，先覆盖 topic 创建、WS topic 事件、topic dialog 与 summary 恢复并观察失败
- [ ] T067 [US3] 在 `crates/helix-im/src/event/channel.rs`、`crates/helix-im/src/ws/handlers/update_channel.rs` 实现 G-15，生成 JSON，删除旧 topic Frame 并更新 ledger
- [ ] T068 [US3] 新增 `crates/helix-im/tests/message_v3/channel/g16/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g16`，先覆盖关闭群或退出群后的 actor/observer lifecycle 与恢复并观察失败
- [ ] T069 [US3] 在 `crates/helix-im/src/event/channel.rs`、`crates/helix-im/src/ws/handlers/channel_close.rs`、`crates/helix-im/src/ws/handlers/channel_member_update.rs` 实现 G-16，生成 JSON，删除旧 lifecycle Frame 并更新 ledger
- [ ] T070 [US3] 新增 `crates/helix-im/tests/message_v3/channel_member/g16b/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g16b`，先覆盖成员退出广播、leaver 与 remaining-member roster 双视角及恢复并观察失败
- [ ] T071 [US3] 在 `crates/helix-im/src/event/channel_member.rs`、`crates/helix-im/src/ws/handlers/channel_member_update.rs` 实现 G-16b，生成 JSON，删除旧 roster Frame 并更新 ledger
- [ ] T072 [US3] 新增 `crates/helix-im/tests/message_v3/channel/g16c/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_g16c`，先覆盖群名称、简介、公告更新、observer 权限差异与恢复并观察失败
- [ ] T073 [US3] 在 `crates/helix-im/src/event/channel.rs`、`crates/helix-im/src/ws/handlers/update_channel.rs`、`crates/helix-im/src/ws/handlers/update_channel_notice.rs` 实现 G-16c，生成 JSON，删除旧 settings Frame 并更新 ledger

**Checkpoint**: channel/member/topic 四个 Gate 不再依赖 Frame/Surface；G-15a 基线仍保持 Green。

## Phase 9: P6 Sync 与 Recovery Gate

**Goal**: 以 MessageV3 事件表达 hello、sync notify、pong gap 与陌生 channel 补全的收敛结果。

- [ ] T074 [US4] 新增 `crates/helix-im/tests/message_v3/sync/gs1/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_gs1`，先覆盖 hello 后 dialog empty→loaded→recovery 并观察失败
- [ ] T075 [US4] 在 `crates/helix-im/src/event/sync.rs`、`crates/helix-im/src/ws/handlers/hello.rs`、`crates/helix-im/src/sync/session.rs` 实现 G-S1，生成 JSON，删除旧 hello Frame 并更新 ledger
- [ ] T076 [US4] 新增 `crates/helix-im/tests/message_v3/sync/gs2/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_gs2`，先覆盖离线 sync notify 收敛和恢复终态并观察失败
- [ ] T077 [US4] 在 `crates/helix-im/src/event/sync.rs`、`crates/helix-im/src/sync/explicit.rs` 实现 G-S2，生成 JSON，删除旧 notify Frame 并更新 ledger
- [ ] T078 [US4] 新增 `crates/helix-im/tests/message_v3/sync/gs3/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_gs3`，先覆盖 pong gap 检测、补偿拉取、收敛与恢复并观察失败
- [ ] T079 [US4] 在 `crates/helix-im/src/event/sync.rs`、`crates/helix-im/src/sync/pong_batch.rs`、`crates/helix-im/src/ws/handlers/pong.rs` 实现 G-S3，生成 JSON，删除旧 gap Frame 并更新 ledger
- [ ] T080 [US4] 新增 `crates/helix-im/tests/message_v3/sync/gs4/main.rs` 并在 `crates/helix-im/Cargo.toml` 注册 `message_v3_gs4`，先覆盖陌生 channel 自动补全、全局同步与恢复并观察失败
- [ ] T081 [US4] 在 `crates/helix-im/src/event/sync.rs`、`crates/helix-im/src/sync/increment.rs`、`crates/helix-im/src/ws/handlers/increment_channel.rs` 实现 G-S4，生成 JSON，删除旧 unknown-channel Frame 并更新 ledger

**Checkpoint**: 四个同步 Gate 只发 MessageV3，恢复终态与在线终态一致。

## Phase 10: P7 全局物理清除与证据收口

**Goal**: 最后一个引用 Gate 完成后，删除共享旧基础并证明仓内只剩 MessageV3 真源。

- [ ] T082 [US5] 删除 `crates/helix-im/src/interaction_frame.rs`、`crates/helix-im/src/surface_composer.rs`、`crates/helix-im/src/surface_runtime.rs`、`crates/helix-im/src/dialog_frame.rs`、`crates/helix-im/src/dialog_patch.rs` 与 `crates/helix-im/src/projector/`，把仍属业务算法的代码迁入领域 owner
- [ ] T083 [US5] 更新 `crates/helix-im/src/lib.rs` 与所有模块引用，移除 Frame/Surface/Projector 导出并保持 `helix-core`、native、FFI、Web 的 `DomainEventBytes` 边界不变
- [ ] T084 [US5] 删除 `crates/helix-im/src/ws/handlers/legacy_event.rs` 及 V1 parser、subscription、alias、fallback；保留现役 Go WS wire parser，并在 `crates/helix-im/src/ws/handlers/mod.rs` 明确两者边界
- [ ] T085 [US5] 清理 `crates/helix-im/src/acl/`、`crates/helix-im/src/query/render_ready/` 与 `crates/helix-im/src/client_api/` 中仅服务 Frame/Surface 的 capabilities、wrapper 和 action 组装
- [ ] T086 [US5] 清理 `crates/helix-im/tests/` 中旧 Frame/Surface/V1 fixture 与断言，并确保 `crates/helix-im/tests/message_v3/<domain>/<gate>/main.rs` 与 `Cargo.toml` 每个 Gate 恰好一个显式 MessageV3 target
- [ ] T087 [US5] 运行 `specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs`，生成精确 30 Gate 的 inbound、sender outbound、receiver outbound 与 provenance，并校验 schema/hash/去敏
- [ ] T088 [US5] 运行 `cargo check -p helix-im`、`cargo test -p helix-im` 及计划中的三端 Rust 验收，把 deterministic/contract 结果写入 `specs/047-im-event-gate-inventory/verification.md`
- [ ] T089 [US5] 运行 `specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs`，证明 Helix 生产源码、当前测试、当前 Matrix/合同零 V2 Frame/Surface、零旧 V1 兼容引用且 30 Gate 全部 MIGRATED；历史归档只允许显式 allowlist
- [ ] T090 [US5] 执行 `git diff --check`、跨仓零修改检查与 `.v1` git-ignore 检查，在 `specs/047-im-event-gate-inventory/review-ledger.json` 记录 PC/Mobile 为 `NOT_SYNCED_BY_SCOPE`，WDIO/Mobile/runtime/dual-user/canonical 证据为 NOT_RUN

**Final Checkpoint**: Helix 生产代码与真实 Gate 测试完成 MessageV3 硬切；`.v1` 是可复现、
被忽略的本地产物。PC/Mobile 同步和实际消费仍明确延期，不冒充完成。

## Dependencies & Execution Order

### Phase Dependencies

1. **P0 治理**: T001-T008。
2. **P0 公共基础**: T009-T018，依赖治理完成。
3. **分母锁定**: T019-T021，依赖公共基础。
4. **P1 基线**: T022-T025，固定顺序 G-15a→G-01。
5. **P2 Post**: T026-T043，依赖 G-01；每个 Gate 严格 test→implementation。
6. **P3 Schedule**: T044-T047，依赖 G-01。
7. **P4 Timeline/Read**: T048-T065，依赖 G-01 与公共持久化释放规则。
8. **P5 Channel**: T066-T073，依赖 G-15a。
9. **P6 Sync**: T074-T081，依赖前述领域 Gate 的稳定事件。
10. **P7 清除**: T082-T090，依赖全部 30 Gate 进入 `MIGRATED` 前一状态。

### Within Each Gate

1. 新合同测试调用真实 `ImModule::handle`，不得只测 sample builder。
2. 先观察测试因缺少 MessageV3 行为而失败。
3. 实现必须在本地持久提交和读回后 Emit。
4. sender/receiver 各自产出 persona-local 事件；没有 receiver 的 Gate 也要生成明确空数组。
5. 删除该 Gate 的 Frame/Surface/V1 路径，不双写。
6. 生成 JSON、校验 schema/hash、更新迁移 ledger、跑聚焦测试。

### Parallel Opportunities

本次选择严格串行方案 A。文档和 schema 虽可物理并行，但为避免迁移清单、ledger 与公共 owner
冲突，不标注 `[P]`。后续只有在用户重新批准并行策略且任务所有权无重叠时才可改动。

## Implementation Strategy

### First Reviewable Slice

1. 完成 T001-T021，得到可执行公共基础与精确分母。
2. 完成 T022-T025，用 G-15a/G-01 验证 channel/post 两条纵切。
3. 停下审阅生成 JSON、持久化 lineage、persona 差异和删除清单。

### Incremental Delivery

每个 Gate 都是独立可验收批次：红测试 → 实现 → 本地提交 → MessageV3 Emit → 删除旧路径 →
generated JSON → focused Green。批次失败则标记 `BLOCKED`，不得推进其依赖 Gate，也不得用
## Loop Task Contracts

> 每个 checkbox 恰好一行。`Validator` 是最低验证，不代表 runtime、dual-user 或 canonical
> 已完成。

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-040 | P0 治理 | `docs/GOAL.md` | Constitution 2.0.0 | 禁用词扫描 | GOAL 仅述 MessageV3 |
| T002 | FR-040 | P0 治理 | `docs/harness/decisions.md` | T001 | ADR 状态检查 | ADR-015 superseded |
| T003 | FR-040,FR-047 | P0 治理 | `docs/ROADMAP.md` | T002 | P0-P7 文本检查 | 活账已更新 |
| T004 | FR-040 | P0 治理 | `AGENTS.md` | T003 | 指令禁用词扫描 | 开局指令一致 |
| T005 | FR-042,FR-046,FR-047 | P0 清单 | `contracts/gate-migration.v1.json` | T004 | schema+30 集合+非空 owner/引用 | 预编码清单完整 |
| T006 | FR-038,FR-046,SC-015 | P0 护栏 | `scripts/check-message-v3-cutover.mjs` | T005 | 破坏性样例证伪+双 ledger 单活跃断言 | checker 可证伪 |
| T007 | FR-027-FR-031 | P0 证据 | `verification.md` | T005 | 分级字段检查 | 六级证据分离 |
| T008 | FR-001-FR-004 | P0 分母 | discovery+ledger | T005 | Gate 差集为空 | 范围一致 |
| T009 | FR-012,FR-037,FR-038 | P0 红测 | event contract test | T008 | envelope+Persist barrier 预期失败 | 红因正确 |
| T010 | FR-043,FR-045 | P0 红测 | artifact schema test | T009 | 测试预期失败 | 红因正确 |
| T011 | FR-046 | P0 红测 | registry test | T010 | 测试预期失败 | 红因正确 |
| T012 | FR-005,FR-037 | P0 事件模块 | `src/event/*` | T011 | event contract test | 七域仅 event/data |
| T013 | FR-043 | P0 测试工具 | artifact helper | T012 | helper unit test | 真 Effect 可序列化 |
| T014 | FR-012,FR-041 | P0 释放规则 | module+port_reply | T013 | PersistOk 顺序断言 | terminal 延后释放 |
| T015 | FR-043-FR-045 | P0 生成器 | generator script | T014 | 禁 builder 扫描 | 仅编排 Rust 产物 |
| T016 | FR-043 | P0 确定性 | schema test | T015 | 重复 hash 相等 | byte 稳定 |
| T017 | FR-042,FR-048 | P0 删除清单 | migration manifest | T016 | 30 Gate 无未知 owner/空清单 | 清单冻结 |
| T018 | FR-037,FR-043,SC-010 | P0 验证 | verification | T017 | P0 聚焦命令 | 基础 Green |
| T019 | FR-005,FR-046 | US1 分母 | registry test | T018 | 30 集合+domain/package 映射 | 测试注册一致 |
| T020 | FR-043,FR-046 | US1 产物 | cutover checker | T019 | 目录集合相等 | generated 分母一致 |
| T021 | FR-027-FR-031 | US1 证据 | verification | T020 | 状态枚举检查 | 缺口未漂绿 |
| T022 | FR-021,FR-041 | P1 G-15a 红测 | G-15a test | T021 | 预期失败 | 双视角红因正确 |
| T023 | FR-021,FR-042 | P1 G-15a | channel owners | T022 | G-15a focused test | G-15a MIGRATED |
| T024 | FR-014,FR-016A,FR-041 | P1 G-01 红测 | G-01 test | T023 | 预期失败 | unread/lastPost 红因正确 |
| T025 | FR-016A,FR-042 | P1 G-01 | post owners | T024 | G-01 focused test | G-01 MIGRATED |
| T026 | FR-016,FR-041 | P2 G-02 红测 | G-02 test | T025 | 预期失败 | 文档消息红因正确 |
| T027 | FR-016,FR-042 | P2 G-02 | post event | T026 | G-02 focused test | G-02 MIGRATED |
| T028 | FR-015,FR-041 | P2 G-03a 红测 | G-03a test | T027 | 预期失败 | retry 红因正确 |
| T029 | FR-015,FR-042 | P2 G-03a | retry path | T028 | G-03a focused test | G-03a MIGRATED |
| T030 | FR-017A-FR-017B | P2 G-04 红测 | G-04 test | T029 | 预期失败 | revoke 红因正确 |
| T031 | FR-017A-FR-017B | P2 G-04 | revoke owners | T030 | G-04 focused test | G-04 MIGRATED |
| T032 | FR-013,FR-017A | P2 G-04a 红测 | G-04a test | T031 | 预期失败 | replay 红因正确 |
| T033 | FR-013,FR-042 | P2 G-04a | sync post | T032 | G-04a focused test | G-04a MIGRATED |
| T034 | FR-010,FR-016 | P2 G-05 红测 | G-05 test | T033 | 预期失败 | forward 红因正确 |
| T035 | FR-010,FR-042 | P2 G-05 | forward path | T034 | G-05 focused test | G-05 MIGRATED |
| T036 | FR-017C,FR-041 | P2 G-06 红测 | G-06 test | T035 | 预期失败 | reaction 红因正确 |
| T037 | FR-017C,FR-042 | P2 G-06 | post update | T036 | G-06 focused test | G-06 MIGRATED |
| T038 | FR-010,FR-017 | P2 G-07 红测 | G-07 test | T037 | 预期失败 | urgent 红因正确 |
| T039 | FR-010,FR-042 | P2 G-07 | urgent update | T038 | G-07 focused test | G-07 MIGRATED |
| T040 | FR-020,FR-041 | P2 G-14 红测 | G-14 test | T039 | 预期失败 | receipt 红因正确 |
| T041 | FR-020,FR-042 | P2 G-14 | read owners | T040 | G-14 focused test | G-14 MIGRATED |
| T042 | FR-017,FR-041 | P2 G-14a 红测 | G-14a test | T041 | 预期失败 | confirm 红因正确 |
| T043 | FR-017,FR-042 | P2 G-14a | template update | T042 | G-14a focused test | G-14a MIGRATED |
| T044 | FR-018,FR-041 | P3 G-08 红测 | G-08 test | T043 | 预期失败 | creator-only 红因正确 |
| T045 | FR-018,FR-042 | P3 G-08 | schedule create | T044 | G-08 focused test | G-08 MIGRATED |
| T046 | FR-018,FR-041 | P3 G-09 红测 | G-09 test | T045 | 预期失败 | cancel 红因正确 |
| T047 | FR-018,FR-042 | P3 G-09 | schedule cancel | T046 | G-09 focused test | G-09 MIGRATED |
| T048 | FR-025,FR-041 | P4 G-10 红测 | G-10 test | T047 | 预期失败 | ready 红因正确 |
| T049 | FR-025,FR-042 | P4 G-10 | timeline ready | T048 | G-10 focused test | G-10 MIGRATED |
| T050 | FR-025,FR-041 | P4 G-10a 红测 | G-10a test | T049 | 预期失败 | older 红因正确 |
| T051 | FR-025,FR-042 | P4 G-10a | older context | T050 | G-10a focused test | G-10a MIGRATED |
| T052 | FR-025,FR-041 | P4 G-10b 红测 | G-10b test | T051 | 预期失败 | newer 红因正确 |
| T053 | FR-025,FR-042 | P4 G-10b | newer timeline | T052 | G-10b focused test | G-10b MIGRATED |
| T054 | FR-026,FR-041 | P4 G-10c 红测 | G-10c test | T053 | 预期失败 | append 红因正确 |
| T055 | FR-026,FR-042 | P4 G-10c | append path | T054 | G-10c focused test | G-10c MIGRATED |
| T056 | FR-025,FR-026 | P4 G-10d 红测 | G-10d test | T055 | 预期失败 | anchor 红因正确 |
| T057 | FR-014,FR-025,FR-042 | P4 G-10d | anchor path | T056 | G-10d focused test | G-10d MIGRATED |
| T058 | FR-025,FR-041 | P4 G-11 红测 | G-11 test | T057 | 预期失败 | locate 红因正确 |
| T059 | FR-025,FR-042 | P4 G-11 | locate path | T058 | G-11 focused test | G-11 MIGRATED |
| T060 | FR-025,FR-041 | P4 G-11a 红测 | G-11a test | T059 | 预期失败 | 双向分页红因正确 |
| T061 | FR-025,FR-042 | P4 G-11a | locate paging | T060 | G-11a focused test | G-11a MIGRATED |
| T062 | FR-025,FR-041 | P4 G-12 红测 | G-12 test | T061 | 预期失败 | thread 红因正确 |
| T063 | FR-025,FR-042 | P4 G-12 | thread path | T062 | G-12 focused test | G-12 MIGRATED |
| T064 | FR-014,FR-019,FR-041 | P4 G-13 红测 | G-13 test | T063 | 预期失败 | viewer read 红因正确 |
| T065 | FR-019,FR-042 | P4 G-13 | dialog read | T064 | G-13 focused test | G-13 MIGRATED |
| T066 | FR-021,FR-041 | P5 G-15 红测 | G-15 test | T065 | 预期失败 | topic 红因正确 |
| T067 | FR-021,FR-042 | P5 G-15 | channel topic | T066 | G-15 focused test | G-15 MIGRATED |
| T068 | FR-022,FR-023 | P5 G-16 红测 | G-16 test | T067 | 预期失败 | lifecycle 红因正确 |
| T069 | FR-022,FR-023 | P5 G-16 | lifecycle owners | T068 | G-16 focused test | G-16 MIGRATED |
| T070 | FR-022,FR-041 | P5 G-16b 红测 | G-16b test | T069 | 预期失败 | roster 红因正确 |
| T071 | FR-022,FR-042 | P5 G-16b | member update | T070 | G-16b focused test | G-16b MIGRATED |
| T072 | FR-024,FR-041 | P5 G-16c 红测 | G-16c test | T071 | 预期失败 | settings 红因正确 |
| T073 | FR-024,FR-042 | P5 G-16c | settings update | T072 | G-16c focused test | G-16c MIGRATED |
| T074 | FR-013,FR-041 | P6 G-S1 红测 | G-S1 test | T073 | 预期失败 | hello 红因正确 |
| T075 | FR-013,FR-042 | P6 G-S1 | hello sync | T074 | G-S1 focused test | G-S1 MIGRATED |
| T076 | FR-013,FR-041 | P6 G-S2 红测 | G-S2 test | T075 | 预期失败 | notify 红因正确 |
| T077 | FR-013,FR-042 | P6 G-S2 | explicit sync | T076 | G-S2 focused test | G-S2 MIGRATED |
| T078 | FR-013,FR-041 | P6 G-S3 红测 | G-S3 test | T077 | 预期失败 | gap 红因正确 |
| T079 | FR-013,FR-042 | P6 G-S3 | pong gap | T078 | G-S3 focused test | G-S3 MIGRATED |
| T080 | FR-013,FR-041 | P6 G-S4 红测 | G-S4 test | T079 | 预期失败 | 补全红因正确 |
| T081 | FR-013,FR-042 | P6 G-S4 | channel fill | T080 | G-S4 focused test | G-S4 MIGRATED |
| T082 | FR-038,FR-048 | P7 删除 | Frame/Surface 源码 | T081 | 零引用扫描 | 共享 V2 删除 |
| T083 | FR-037,FR-040 | P7 导出 | lib+driver boundary | T082 | cargo check | 仅 MessageV3 导出 |
| T084 | FR-038 | P7 V1 删除 | WS legacy path | T083 | alias/fallback 扫描 | 旧 V1 删除 |
| T085 | FR-038,FR-042 | P7 清理 | acl/query/client_api | T084 | capabilities/wrapper 扫描 | 套壳清除 |
| T086 | FR-038,FR-046 | P7 测试清理 | helix-im tests | T085 | registry test | 一 Gate 一目标 |
| T087 | FR-036A,FR-043-FR-046 | P7 产物 | `.v1` generated | T086 | schema/hash/30 集合 | 产物完整 |
| T088 | SC-009-SC-012 | P7 Rust 验证 | workspace crates | T087 | cargo 命令 | Rust 证据记录 |
| T089 | SC-009,SC-013 | P7 零引用 | cutover checker | T088 | Helix scoped checker pass | 全部 MIGRATED |
| T090 | FR-044,SC-014 | P7 收口 | diff+ledger | T089 | diff/ignore/跨仓检查 | Helix-only 完成、客户端未同步 |
