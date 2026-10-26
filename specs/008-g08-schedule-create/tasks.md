# Tasks: G08 创建与读取定时消息

**Loop Profile**: `helix-loop/v1` | **Final Native owner**: `../017-uc04-16-native-e2e/`

## Phase 1: Governance and Isolation

- [X] T001 [UC09] 校验 discovery/data-model/interaction contract、CURRENT/TARGET/GAP 与 Constitution Check；冻结 Helix/Go/client revision、dirty state、独立 worktree 和 owner file set。
- [X] T002 [UC09] 在 `crates/helix-im/tests/g08_schedule_create_interaction_test.rs` 先写可失败的 action/Tick/authority/PersistOk/Frame replay；必须覆盖：创建等待 WS；attach 用 owner-only getSchedule hydrate；非 owner 数据拒绝。
- [X] T003 [UC09] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.spec.ts` 先写可失败的 binding/whole-frame/DOM/ARIA 测试，固定 定时草稿、owner hydrate、提交与失败恢复 的 ready/failed/forbidden/empty 状态。

## Phase 2: Helix Vertical Slice

- [X] T004 [UC09] 在 `crates/helix-im/src/projector/action.rs` 接入 scheduleMessage/getSchedule hydrate capability，校验 actor/scope/epoch/revision/input 并补 HOP correlation。
- [X] T005 [UC09] 在 `crates/helix-im/src/ws/handlers/post_schedule_created.rs` 实现/复用 canonical adapter 与 authority normalize；HTTP SUCCESS 不建立 mutation 完成。
- [X] T006 [UC09] 在 `crates/helix-im/src/acl/storage_effects.rs` 构造本 Gate 的单批 `PersistAtomic`，将 entity/viewer index/cursor 或 eventSeq 一起提交。
- [X] T007 [UC09] 在 `crates/helix-im/src/projector/composer.rs` 与 `crates/helix-im/src/interaction_frame.rs` 仅于匹配 `PersistOk` 后合成受影响 Surface 的完整 VM/row 和单调 Frame。

## Phase 3: PC Consumer and UI

- [X] T008 [UC09] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/surface-action-invocation.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts` 同步 typed binding/codec/whole-frame apply；禁止 raw event/HTTP reducer。
- [X] T009 [UC09] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.scss` 实现 定时草稿、owner hydrate、提交与失败恢复，对齐 Index-Dual 的状态、交互、ARIA 与稳定 data-*。

## Phase 4: Focused Readiness

- [X] T010 [UC09] 批量运行 T002/T003 的 focused Rust/Angular tests；记录实际 start/end/wall-clock，禁止本 Gate 单独 Native build。
- [X] T011 [UC09] 运行 Spec Kit R1-R3 与 analyze，确认 task contract/requirement 覆盖完整；同一失败签名两次仍失败则 NeedsHuman。
- [X] T012 [UC09] 将 actor/observer、Go/PG、SQLite、Frame/DOM/visual 最终断言登记到 017，并且只在 focused evidence 完整时标 `READY_FOR_FINAL_E2E`。

## Dependencies

`T001 -> T002,T003 -> T004 -> T005 -> T006 -> T007 -> T008 -> T009 -> T010 -> T011 -> T012`。T002/T003 必须在任何生产实现前建立红灯；T010 只跑 focused tests，Native 全批只由 017 执行。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-001,FR-007 | I-001 | `specs/008-g08-schedule-create/contracts/008-g08-schedule-create-interaction.md` + 三仓 worktree manifest | discovery 已完成 | V-003 | 状态/owner/worktree 可审计 |
| T002 | FR-001..FR-005 | V-001 | `crates/helix-im/tests/g08_schedule_create_interaction_test.rs` | T001 | V-001 red test | 破坏任一因果闸门即失败 |
| T003 | FR-005,FR-006 | V-002 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.spec.ts` | T001 | V-002 red test | 旧 reducer/partial Frame 无法通过 |
| T004 | FR-001,FR-002 | A-001 | `crates/helix-im/src/projector/action.rs` | T002 red | V-001 | capability 严格且 stale fail-closed |
| T005 | FR-002,FR-003 | A-001 | `crates/helix-im/src/ws/handlers/post_schedule_created.rs` | T002,T004 | V-001 | canonical Effect/authority 映射成立 |
| T006 | FR-003 | S-001 | `crates/helix-im/src/acl/storage_effects.rs` | T005 | V-001 | write-set 原子且幂等 |
| T007 | FR-004,FR-005 | A-001 | `crates/helix-im/src/projector/composer.rs` + `crates/helix-im/src/interaction_frame.rs` | T006 | V-001 | PersistOk 后完整 Frame |
| T008 | FR-001,FR-005 | A-001 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/surface-action-invocation.ts` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts` | T003,T007 | V-002 | typed invoke/atomic apply 且无 raw reducer |
| T009 | FR-006 | A-001 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.ts` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.scss` | T003,T008 | V-002 | 定时草稿、owner hydrate、提交与失败恢复 可交互可恢复 |
| T010 | FR-007 | V-001,V-002 | `crates/helix-im/tests/g08_schedule_create_interaction_test.rs` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.ts` | T002-T009 | focused command outputs | focused tests PASS 且无 Native build |
| T011 | FR-007 | V-003 | `specs/008-g08-schedule-create/review-ledger.json` | T010 | loop_engine + speckit-analyze | R1-R3 current PASS 且 CRITICAL=0 |
| T012 | FR-007 | V-004 | `specs/017-uc04-16-native-e2e/tasks.md` readiness row | T011 | 017 manifest validator | final assertions 完整且状态仅 readiness |

