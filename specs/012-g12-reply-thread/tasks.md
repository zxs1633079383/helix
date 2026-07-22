# Tasks: G12 回复树与分支

**Loop Profile**: `helix-loop/v1` | **Final Native owner**: `../017-uc04-16-native-e2e/`

## Phase 1: Governance and Isolation

- [ ] T001 [UC12] 校验 discovery/data-model/interaction contract、CURRENT/TARGET/GAP 与 Constitution Check；冻结 Helix/Go/client revision、dirty state、独立 worktree 和 owner file set。
- [ ] T002 [UC12] 在 `crates/helix-im/tests/g12_reply_thread_interaction_test.rs` 先写可失败的 action/Tick/authority/PersistOk/Frame replay；必须覆盖：G12 只拥有 hydration/relation；reply mutation 复用 G01 authority。
- [ ] T003 [UC12] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-reply-drawer.component.spec.ts` 先写可失败的 binding/whole-frame/DOM/ARIA 测试，固定 root/reply tree、分页、drawer 回复与 summary 的 ready/failed/forbidden/empty 状态。

## Phase 2: Helix Vertical Slice

- [ ] T004 [UC12] 在 `crates/helix-im/src/projector/action.rs` 接入 openThread/loadReplyBranch + replyMessage adapter capability，校验 actor/scope/epoch/revision/input 并补 HOP correlation。
- [ ] T005 [UC12] 在 `crates/helix-im/src/query/render_ready/replies.rs` 实现/复用 canonical adapter 与 authority normalize；HTTP SUCCESS 不建立 mutation 完成。
- [ ] T006 [UC12] 在 `crates/helix-im/src/acl/storage_effects.rs` 构造本 Gate 的单批 `PersistAtomic`，将 entity/viewer index/cursor 或 eventSeq 一起提交。
- [ ] T007 [UC12] 在 `crates/helix-im/src/projector/thread_detail.rs` 与 `crates/helix-im/src/interaction_frame.rs` 仅于匹配 `PersistOk` 后合成受影响 Surface 的完整 VM/row 和单调 Frame。

## Phase 3: PC Consumer and UI

- [ ] T008 [UC12] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/surface-action-invocation.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts` 同步 typed binding/codec/whole-frame apply；禁止 raw event/HTTP reducer。
- [ ] T009 [UC12] 在 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-reply-drawer.component.ts` 与 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.scss` 实现 root/reply tree、分页、drawer 回复与 summary，对齐 Index-Dual 的状态、交互、ARIA 与稳定 data-*。

## Phase 4: Focused Readiness

- [ ] T010 [UC12] 批量运行 T002/T003 的 focused Rust/Angular tests；记录实际 start/end/wall-clock，禁止本 Gate 单独 Native build。
- [ ] T011 [UC12] 运行 Spec Kit R1-R3 与 analyze，确认 task contract/requirement 覆盖完整；同一失败签名两次仍失败则 NeedsHuman。
- [ ] T012 [UC12] 将 actor/observer、Go/PG、SQLite、Frame/DOM/visual 最终断言登记到 017，并且只在 focused evidence 完整时标 `READY_FOR_FINAL_E2E`。

## Dependencies

`T001 -> T002,T003 -> T004 -> T005 -> T006 -> T007 -> T008 -> T009 -> T010 -> T011 -> T012`。T002/T003 必须在任何生产实现前建立红灯；T010 只跑 focused tests，Native 全批只由 017 执行。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-001,FR-007 | I-001 | `specs/012-g12-reply-thread/contracts/012-g12-reply-thread-interaction.md` + 三仓 worktree manifest | discovery 已完成 | V-003 | 状态/owner/worktree 可审计 |
| T002 | FR-001..FR-005 | V-001 | `crates/helix-im/tests/g12_reply_thread_interaction_test.rs` | T001 | V-001 red test | 破坏任一因果闸门即失败 |
| T003 | FR-005,FR-006 | V-002 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-reply-drawer.component.spec.ts` | T001 | V-002 red test | 旧 reducer/partial Frame 无法通过 |
| T004 | FR-001,FR-002 | A-001 | `crates/helix-im/src/projector/action.rs` | T002 red | V-001 | capability 严格且 stale fail-closed |
| T005 | FR-002,FR-003 | A-001 | `crates/helix-im/src/query/render_ready/replies.rs` | T002,T004 | V-001 | canonical Effect/authority 映射成立 |
| T006 | FR-003 | S-001 | `crates/helix-im/src/acl/storage_effects.rs` | T005 | V-001 | write-set 原子且幂等 |
| T007 | FR-004,FR-005 | A-001 | `crates/helix-im/src/projector/thread_detail.rs` + `crates/helix-im/src/interaction_frame.rs` | T006 | V-001 | PersistOk 后完整 Frame |
| T008 | FR-001,FR-005 | A-001 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/surface-action-invocation.ts` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts` | T003,T007 | V-002 | typed invoke/atomic apply 且无 raw reducer |
| T009 | FR-006 | A-001 | `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-reply-drawer.component.ts` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.scss` | T003,T008 | V-002 | root/reply tree、分页、drawer 回复与 summary 可交互可恢复 |
| T010 | FR-007 | V-001,V-002 | `crates/helix-im/tests/g12_reply_thread_interaction_test.rs` + `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-reply-drawer.component.ts` | T002-T009 | focused command outputs | focused tests PASS 且无 Native build |
| T011 | FR-007 | V-003 | `specs/012-g12-reply-thread/review-ledger.json` | T010 | loop_engine + speckit-analyze | R1-R3 current PASS 且 CRITICAL=0 |
| T012 | FR-007 | V-004 | `specs/017-uc04-16-native-e2e/tasks.md` readiness row | T011 | 017 manifest validator | final assertions 完整且状态仅 readiness |

