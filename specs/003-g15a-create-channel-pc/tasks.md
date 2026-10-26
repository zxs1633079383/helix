# Tasks: G15a PC 创建群聊

**Input**: Design documents from `/specs/003-g15a-create-channel-pc/`

**Prerequisites**: `plan.md`、`spec.md`、`research.md`、`data-model.md`、`contracts/`

**Tests**: 本功能要求测试先行。每个 story 的测试任务必须先写、先运行并确认能因目标实现缺失而失败，再进入实现任务。

**Organization**: 任务按用户故事组织；共享 InteractionFrame schema 与 action-ref 基础放在 Foundational。所有 PC 路径均指向 `cses-client`，没有移动端任务。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可在不同文件上并行执行，且不依赖同阶段尚未完成任务
- **[Story]**: 映射 `spec.md` 的 US1/US2/US3
- 每个任务均含精确文件路径，并在文末 Loop Task Contracts 中恰好出现一次

## Phase 1: Setup (Shared Test Fixtures)

**Purpose**: 先建立 Rust 与 PC 共同理解的 G15a contract fixture，避免两侧各自发明 payload。

- [ ] T001 在 `crates/helix-im/tests/fixtures/interaction_v2/g15a-create-channel.json` 建立根级 `channel.create` binding、合法 invocation、PersistOk 后 dialog mutation 与失败 receipt 的 canonical fixture，并在 `crates/helix-im/tests/fixtures/interaction_v2/README.md` 登记唯一真源

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 扩展 typed DialogList contract，并让 Rust/TypeScript 在进入任一用户故事前先对齐。

**CRITICAL**: 本阶段未完成，不得开始 story 实现。

- [ ] T002 [P] 在 `crates/helix-im/tests/interaction_frame_contract_test.rs` 先增加失败测试，证明 `DialogListVm.actions` 必须通过 whole-frame validation 且 stale/空 action ref fail-closed
- [ ] T003 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-contract.codec.test.ts` 先增加失败测试，证明 PC codec 接受根级 actions、拒绝畸形 binding 且不丢失 actionRef
- [ ] T004 在 `crates/helix-im/src/interaction_frame.rs` 与 `crates/helix-im/src/dialog_frame.rs` 增加 `DialogListVm.actions`，并从 `crates/helix-im/src/projector/action.rs` 抽取可供多 surface 复用的确定性 opaque action-ref allocator，使 T002 通过
- [ ] T005 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/left-dialog-list-frame.store.ts` 增加根级 actions 的类型、decode 与原子 cache 保留逻辑，使 T003 通过
- [ ] T006 同步 `crates/helix-im/tests/interaction_frame_fixture_manifest_test.rs` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.fixture-parity.test.ts` 对 T001 fixture 的清单和跨语言 parity，禁止 Rust/PC schema 分叉

**Checkpoint**: DialogList 根级 capability 已是可验证的 typed contract，但尚未发送建群 HTTP。

---

## Phase 3: User Story 1 - 创建者建立并进入可信群聊 (Priority: P1) MVP

**Goal**: PC 创建者从根级 capability 打开本地 overlay，提交一次合法请求；权威事实经 PersistAtomic/PersistOk 后只形成一个可进入群聊。

**Independent Test**: 一个创建者选择至少一名成员并提交，断言一个 HTTP Effect、PersistOk 前零成功 Frame、PersistOk 后一条 dialog row；PC 只消费 binding 与 Frame，进入后取得 summary + composer bundle。

### Tests for User Story 1

- [ ] T007 [P] [US1] 在 `crates/helix-im/tests/channel_create_action_test.rs` 先增加失败测试，覆盖根级 binding 注册、合法 ActionInvocation 到唯一 canonical HTTP Effect、creator 排除与成员确定性去重
- [ ] T008 [P] [US1] 在 `crates/helix-im/tests/channel_create_persist_barrier_test.rs` 先增加失败 replay，断言 HTTP/WS 权威事实只发一个 PersistAtomic、PersistOk 前无 Frame、匹配 PersistOk 后只有一个带 causation id 的 dialog mutation
- [ ] T009 [P] [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts` 先增加失败 DOM 测试，覆盖 binding 打开 overlay、合法 draft 携原 binding 提交、submitting 防双击和 Frame 到达后唯一行

### Implementation for User Story 1

- [ ] T010 [US1] 在 `crates/helix-im/src/projector/action.rs` 与 `crates/helix-im/src/module/action.rs` 增加 `CreateChannel` dispatch、严格 SurfaceData 输入解析和 actor/scope/epoch/revision 校验，并调用现有 `im_create_channel`
- [ ] T011 [US1] 在 `crates/helix-im/src/dialog_frame.rs` 与 `crates/helix-im/src/module/action.rs` 为已授权 `message.left-dialog-list@User` 注册 `channel.create` binding，并把 request/correlation identity 写入现有 module state
- [ ] T012 [US1] 在新文件 `crates/helix-im/src/channel/create_commit.rs` 定义 `AuthoritativeChannelCreated`、`ChannelCreatedCommit` 与单批 channel/member/viewer-state `PersistAtomic` 生成器，并从 `crates/helix-im/src/channel/mod.rs` 导出
- [ ] T013 [US1] 在 `crates/helix-im/src/port_reply/dispatch_channel_create.rs`、`crates/helix-im/src/ws/handlers/channel_created.rs` 与 `crates/helix-im/src/port_reply/dispatch.rs` 统一接入 T012，并只在匹配 PersistOk 时 patch DialogList Frame 和结束创建者 action
- [ ] T014 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-create-chat.component.ts` 接入现有候选成员数据源，保持名称、成员、loading 与 draft 为 PC 本地状态，并输出仅含 `display_name`/`user_ids` 的规范输入
- [ ] T015 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-surface-runtime-view.component.ts` 接入 create overlay，以 typed `channel.create` binding 打开并通过现有 `SurfaceActionRequested` 提交，不在组件内直接调用业务 command
- [ ] T016 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/shell.service.ts` 移除或封闭 message-v4 旧 direct `createChat` 入口，并在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/__tests__/message-v4-boundary.test.ts` 加硬切断言禁止 raw/direct fallback
- [ ] T017 [US1] 在 `crates/helix-im/tests/dialog_projection_test.rs` 增加创建后 EnterChannel 回归，证明新 channel row 进入既有 GS04 summary + composer bundle 且 timeline hydration 不阻塞 bundle

**Checkpoint**: US1 可独立演示；只完成“创建者合法创建”就形成 MVP，失败与多用户恢复尚未放行生产验收。

---

## Phase 4: User Story 2 - 失败不会制造幽灵群聊 (Priority: P2)

**Goal**: stale capability、非法草稿、远端拒绝、无效结果与 PersistErr 均保持上一致 Frame，PC 保留可修正 draft。

**Independent Test**: 依次注入五类失败，断言非法输入零 HTTP Effect，其余失败零成功 Frame/零新 row，receipt 与 request 相关，overlay 解除 submitting 但保留输入。

### Tests for User Story 2

- [ ] T018 [P] [US2] 在 `crates/helix-im/tests/channel_create_action_test.rs` 先增加失败矩阵，覆盖 stale/cross-actor binding、空/超长名称、空/非法成员、额外字段、HTTP reject、无效 envelope 与 PersistErr
- [ ] T019 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts` 先增加失败 DOM 矩阵，证明 validation/reject/persist failure 保留 draft、解除 submitting、显示关联错误且不插入 dialog row

### Implementation for User Story 2

- [ ] T020 [US2] 在 `crates/helix-im/src/module/action.rs` 与 `crates/helix-im/src/error.rs` 实现 create-channel 输入/能力拒绝类型和无 HTTP Effect 的 terminal receipt，外部输入路径不使用 unwrap/expect
- [ ] T021 [US2] 在 `crates/helix-im/src/port_reply/dispatch_channel_create.rs` 与 `crates/helix-im/src/channel/create_commit.rs` 实现 HTTP/authority/PersistErr 的关联失败转换，保持 Frame state 不变并在同签名第三次失败时进入 NeedsHuman
- [ ] T022 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-create-chat.component.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-surface-runtime-view.component.ts` 消费 action rejection/directive，保留 draft 并提供修正、重试或人工处理状态

**Checkpoint**: US2 单独验证时，所有失败均可证伪“没有幽灵群聊”。

---

## Phase 5: User Story 3 - 受邀成员与恢复链路独立收敛 (Priority: P3)

**Goal**: HTTP、WS、sync 重复/乱序或断线恢复时，各相关用户独立得到唯一结果，未参与用户零变化。

**Independent Test**: 创建者、受邀者、未参与者三 scope 回放同一 channel fact 的不同顺序，断言前两者各一行/唯一成员关系，第三者无 storage/Frame/capability；detached 恢复不重发 create HTTP。

### Tests for User Story 3

- [ ] T023 [P] [US3] 在 `crates/helix-im/tests/channel_create_persist_barrier_test.rs` 先增加三用户、三来源、重复/乱序、detached mount 与恢复 replay，且加入破坏幂等键或绕过 PersistOk 即失败的可证伪断言
- [ ] T024 [P] [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/__tests__/left-dialog-list-frame.store.test.ts` 先增加重复 Frame、不同 user scope 与未参与 scope 的 cache 隔离测试

### Implementation for User Story 3

- [ ] T025 [US3] 在 `crates/helix-im/src/channel/create_commit.rs`、`crates/helix-im/src/sync/increment.rs` 与 `crates/helix-im/src/dialog_frame.rs` 以 `channel_id + viewer scope` 统一 HTTP/WS/sync 幂等收敛和 detached remount，受邀者不复用创建者 causation/action state
- [ ] T026 [US3] 在 `scripts/host-cli-dual-account-e2e.sh` 增加可选 G15a 三账号场景，执行正常创建、重复事实与断线恢复，只读核对远端 channel/member 和双方独立 Frame correlation，并证明未参与用户零变化

**Checkpoint**: 三个用户故事均可独立验证，跨用户与恢复语义不依赖 PC 临时状态。

---

## Phase 6: Polish & Cross-Cutting Evidence

**Purpose**: 同步合同、发布矩阵和可复现证据，不用历史 fixture 代替当前实现结果。

- [ ] T027 [P] 同步 `crates/helix-im/contracts/command-api.v1.json`、`crates/helix-im/contracts/strict-matrix.v1.json`、`crates/helix-im/tests/projection_schema_test.rs` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.fixture-parity.test.ts` 的 G15a typed binding/Frame 合同，并把 legacy raw emit 标为非完成证据
- [ ] T028 按 `specs/003-g15a-create-channel-pc/quickstart.md` 运行 focused Rust、PC DOM、contract parity、无移动端路径与双用户真链 gate，将每条命令的 start/end/wall-clock/exit/retry/blocker/SHA256 写入 `specs/003-g15a-create-channel-pc/evidence/evidence.json` 和 `specs/003-g15a-create-channel-pc/verification.md`，随后执行 R4 validator

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1**: 无依赖；先固定 canonical fixture。
- **Phase 2**: 依赖 T001；完成后才允许 story 开工。
- **US1**: 依赖 Phase 2；是 MVP。
- **US2**: 依赖 Phase 2，并复用 US1 的 dispatch/commit 接缝；测试可以先写，实现合并在 US1 后。
- **US3**: 依赖 Phase 2，并复用 US1 的 commit path；测试可以与 US2 并行，最终实现需 T012-T013。
- **Phase 6**: 依赖选定交付的全部 story；T028 最后执行。

### Within Each User Story

- 测试任务先完成并记录预期失败，再执行实现任务。
- action schema/dispatch 在 effect 编排之前；commit model 在 HTTP/WS/sync 路由之前。
- Rust Frame 合同通过后再让 PC consumer 接受新字段。
- 任何成功 UI 断言必须以匹配 PersistOk 为前置。

### Parallel Opportunities

- T002 与 T003 可跨仓库并行；T004 与 T005 在各自测试失败后可并行。
- US1 的 T007、T008、T009 可并行写失败测试。
- US2 的 T018 与 T019、US3 的 T023 与 T024 可跨 Rust/PC 并行。
- T027 可在真链环境准备期间进行，但必须在 T028 前完成。

## Parallel Examples

### User Story 1

```text
T007 Rust ActionInvocation contract tests
T008 Rust PersistAtomic/PersistOk replay tests
T009 PC overlay and DOM tests
```

### User Story 2

```text
T018 Rust failure matrix
T019 PC draft-preservation DOM matrix
```

### User Story 3

```text
T023 Rust three-scope recovery replay
T024 PC scope/cache isolation
```

## Implementation Strategy

### MVP First

1. 完成 T001-T006，锁定跨语言 typed contract。
2. 完成 T007-T017，只交付 US1。
3. 跑 focused Rust + PC DOM，确认合法创建链可独立工作。
4. 在 US2/US3 完成前不得把 MVP 证据宣称为完整 G15a 生产验收。

### Incremental Delivery

1. US1：合法创建与进入。
2. US2：失败不制造幽灵群聊。
3. US3：多用户、重复与恢复独立收敛。
4. 最后同步合同并跑 R4 真证据。

## Loop Task Contracts *(mandatory)*

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|------|-------------|-----------|--------------|---------------|-----------|-------------|
| T001 | FR-001, FR-004, FR-007 | I-001, I-002, A-003 | `crates/helix-im/tests/fixtures/interaction_v2/` | R1-R2 PASS | `cargo test -p helix-im --test interaction_frame_fixture_manifest_test` | canonical fixture 被清单唯一引用 |
| T002 | FR-001, FR-005 | V-001 | `crates/helix-im/tests/interaction_frame_contract_test.rs` | T001 | `cargo test -p helix-im --test interaction_frame_contract_test` | 缺失或畸形根级 action 时测试按预期失败 |
| T003 | FR-001, FR-005 | V-004 | `cses-client/.../surface-contract.codec.test.ts` | T001 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/surface-runtime/__tests__/surface-contract.codec.test.ts` | PC codec 失败测试可证伪 binding 合同 |
| T004 | FR-001, FR-005 | R-001, A-001 | `interaction_frame.rs`, `dialog_frame.rs`, `projector/action.rs` | T002 failed as expected | `cargo test -p helix-im --test interaction_frame_contract_test` | Rust whole-frame contract 接受合法根级 action |
| T005 | FR-001, FR-005 | R-001, V-004 | `surface-contract.types.ts`, `left-dialog-list-frame.store.ts` | T003 failed as expected | `yarn test --runInBand --runTestsByPath src/pages/message-v4/surface-runtime/__tests__/surface-contract.codec.test.ts` | PC 原子保留 actionRef |
| T006 | FR-005, FR-015 | V-006 | Rust/PC fixture parity tests | T004, T005 | `cargo test -p helix-im --test interaction_frame_fixture_manifest_test` | 两仓 fixture parity 同时通过 |
| T007 | FR-001, FR-002, FR-003, FR-004 | V-001 | `crates/helix-im/tests/channel_create_action_test.rs` | Phase 2 | `cargo test -p helix-im --test channel_create_action_test` | 合法 invocation 的失败测试证明缺少 dispatch |
| T008 | FR-006, FR-007, FR-008 | V-002 | `crates/helix-im/tests/channel_create_persist_barrier_test.rs` | Phase 2 | `cargo test -p helix-im --test channel_create_persist_barrier_test` | 去掉 PersistOk 时零成功 Frame 断言成立 |
| T009 | FR-002, FR-004, FR-005 | V-004 | PC create overlay/runtime DOM tests | Phase 2 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts` | DOM 测试先因未接 binding 失败 |
| T010 | FR-001, FR-002, FR-003, FR-004 | A-001 | `projector/action.rs`, `module/action.rs` | T007 failed as expected | `cargo test -p helix-im --test channel_create_action_test` | 一次合法 invocation 只产生一个 canonical HTTP Effect |
| T011 | FR-001, FR-004 | I-001, A-001 | `dialog_frame.rs`, `module/action.rs` | T004, T010 | `cargo test -p helix-im --test channel_create_action_test` | 授权 DialogList 发布 actor/scope-bound binding |
| T012 | FR-006, FR-007, FR-008 | R-003, A-002 | `channel/create_commit.rs`, `channel/mod.rs` | T008 failed as expected | `cargo test -p helix-im --test channel_create_persist_barrier_test` | 权威事实只生成一个 PersistAtomic batch |
| T013 | FR-006, FR-007, FR-008 | A-002, A-003, S-004 | HTTP/WS/PersistOk dispatch files | T011, T012 | `cargo test -p helix-im --test channel_create_persist_barrier_test` | PersistOk 后一次 Frame 与 terminal receipt |
| T014 | FR-002, FR-003, FR-005 | R-002, I-002 | `message-ui-create-chat.component.ts` | T009 failed as expected | `yarn test --runInBand --runTestsByPath src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts` | overlay 只输出规范输入且中间态本地化 |
| T015 | FR-001, FR-004, FR-005 | R-001, A-001 | `message-surface-runtime-view.component.ts` | T005, T014 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts` | typed binding 打开并提交 overlay |
| T016 | FR-005, FR-006 | R-002 | `shell.service.ts`, `message-v4-boundary.test.ts` | T015 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/__tests__/message-v4-boundary.test.ts` | message-v4 不存在 direct/raw create fallback |
| T017 | FR-009 | A-005 | `crates/helix-im/tests/dialog_projection_test.rs` | T013 | `cargo test -p helix-im --test dialog_projection_test` | EnterChannel 先取得 summary + composer bundle |
| T018 | FR-001, FR-002, FR-012 | V-001, V-002 | `crates/helix-im/tests/channel_create_action_test.rs` | Phase 2 | `cargo test -p helix-im --test channel_create_action_test` | 七类失败矩阵先因拒绝语义缺失而失败 |
| T019 | FR-012 | V-004 | PC create overlay DOM test | Phase 2 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts` | 五类失败均能可证伪 draft 丢失/幽灵行 |
| T020 | FR-001, FR-002, FR-012 | A-004, S-001 | `module/action.rs`, `error.rs` | T018 failed as expected | `cargo test -p helix-im --test channel_create_action_test` | 本地拒绝零 HTTP Effect 且 receipt 可关联 |
| T021 | FR-006, FR-007, FR-012 | A-004, S-004 | `dispatch_channel_create.rs`, `channel/create_commit.rs` | T012, T018 | `cargo test -p helix-im --test channel_create_persist_barrier_test` | authority/PersistErr 零成功 Frame，第三次同签名 NeedsHuman |
| T022 | FR-012 | A-004 | PC create overlay/runtime components | T019, T020, T021 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/components/__tests__/message-ui-create-chat.component.test.ts` | PC 保留 draft 并显示修正/重试/HITL 状态 |
| T023 | FR-010, FR-011, FR-013 | V-003 | `channel_create_persist_barrier_test.rs` | Phase 2 | `cargo test -p helix-im --test channel_create_persist_barrier_test` | 三 scope/三来源失败 replay 可证伪重复与泄漏 |
| T024 | FR-010, FR-011, FR-013 | V-004 | `left-dialog-list-frame.store.test.ts` | Phase 2 | `yarn test --runInBand --runTestsByPath src/pages/message-v4/__tests__/left-dialog-list-frame.store.test.ts` | cache 隔离测试拒绝跨 scope/重复 row |
| T025 | FR-010, FR-011, FR-013 | R-004, R-005, S-005, S-006 | commit/sync/dialog projection files | T013, T023 | `cargo test -p helix-im --test channel_create_persist_barrier_test` | 每 viewer 唯一收敛，detached remount 不重发 HTTP |
| T026 | FR-010, FR-011, FR-013, FR-014 | V-005 | `scripts/host-cli-dual-account-e2e.sh` | T025, 真 Go 前置可用 | `scripts/host-cli-dual-account-e2e.sh --scenario g15a-create-channel` | 两相关用户各一结果，未参与用户零变化 |
| T027 | FR-006, FR-014, FR-015 | V-006 | Helix contracts/Strict Matrix 与 PC parity test | T013, T022, T025 | `cargo test -p helix-im --test projection_schema_test` | typed G15a 合同同步且 raw emit 非完成证据 |
| T028 | FR-014, FR-015 | V-001, V-002, V-003, V-004, V-005, V-006 | `quickstart.md`, `evidence/`, `verification.md` | T001-T027 | `python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance` | R4 PASS 或如实 blocked，含完整计时与哈希 |
