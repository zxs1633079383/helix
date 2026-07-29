# Feature Specification: G12 回复树与分支

**Feature**: `012-g12-reply-thread` | **Created**: 2026-07-22 | **Status**: Draft | **Macro UC**: UC12

**Input**: 按 Gate 独立完成 回复树与分支 的 Helix -> cses-client 全链路；单 Gate 不运行 Native 全量验收。

**Loop Profile**: `helix-loop/v1`

## User Scenario & Testing

### UC12 - 回复树与分支 (P1)

**Trigger**: 用户打开全部回复、加载下一页/分支，或发送 reply 后收到权威 post。

**Independent Test**: canonical Tick/权威 reply fixture 必须独立证明 ActionBinding、Persist write-set 与 Frame；这是 focused readiness，不是 Native/真实链 Green。

**Acceptance Scenarios**:

1. Given 当前 actor/scope/epoch/revision，When openThread / loadReplies / loadReplyBranch / replyMessage，Then 只生成合法 Tick/Effect，transport ack 不建立成功。
2. Given 校验后的 reply query；新 reply 以 post WS/sync 为权威 到达，When PersistAtomic 成功，Then 发布完整 matching Frame，PC 呈现“右侧回复抽屉/移动 sheet；root、总数、分页、composer；新回复同步主区最多三条预览”。
3. Given 重复、乱序、超时、权限拒绝或 revision conflict，Then 不破坏上一版 VM，并给出可恢复的 rejected/failed 状态。

## Loop Contract

- **Trigger**: 用户打开全部回复、加载下一页/分支，或发送 reply 后收到权威 post。
- **Goal**: 右侧回复抽屉/移动 sheet；root、总数、分页、composer；新回复同步主区最多三条预览。
- **Completion**: 权威事实、远端 DB、本地 PersistOk、完整 Frame 与 PC DOM 因果一致；运行证据在 017 一次性收集。
- **Boundaries**: Angular 不消费 raw WS/HTTP、不 join 业务 Surface；本 feature 不启动 Native E2E。
- **Human Gate**: replyCount 是权威绝对数，不得用当前数组长度；edge lookup O(1)/O(log n)。

### Trigger

用户通过本 Gate 声明的 ActionBinding/HostObservation 发出 intent，或当前 viewer 从 Go WS/sync 接收本 Gate 的权威事实。

### Goal

把该 Gate 的 capability、Tick/Effect、权威 normalize、PersistAtomic、Surface/完整 VM、InteractionFrame 与 Angular/UI 收敛为一条可回放纵切。

### Completion

focused Rust 与 Angular 可证伪测试通过、R1-R3 当前 PASS，并登记 017 所需 actor/observer、PG/SQLite、Frame/DOM/visual 断言；此时只标 READY_FOR_FINAL_E2E。

### Boundaries

本 Gate 不单独运行 Native E2E，不让 Angular 消费 raw WS/HTTP 或跨 Surface join 业务事实，不把平台 overlay/draft/动画写入共享 domain。

### Human Gate

需要改变 Go 领域语义、扩大权限、执行不可逆迁移，或同一 task/validator/失败签名两次自动修复仍失败时进入 NeedsHuman。

## Gate Contract

| 维度 | 合同 |
|---|---|
| ActionBinding | `openThread / loadReplies / loadReplyBranch / replyMessage`，含 actionRef/scope/epoch/revision/capability |
| Tick / Effect | `Tick::Command(ActionInvoked) -> getReplies/getReplyBranch 或发送链 Effect` |
| Authority | 校验后的 reply query；新 reply 以 post WS/sync 为权威 |
| Persist | `PersistAtomic(messages + reply edges + root/page coverage + cursor)`，PersistOk 前零业务 Frame |
| Surface | message.thread-detail、message.timeline、必要时 left-dialog-list |
| VM / Row | 完整 ThreadDetailVm(root/nodes/orderedReplyIds/replyCount/page/state) + root MessageVm.replyThread 最多 3 条 |
| InteractionFrame | 同 PersistOk 原子更新抽屉与主时间线回复摘要；跨页 id 去重稳定排序 |
| Angular | `message-ui-reply-drawer.component.ts；message-ui-surface-thread-drawer.component.ts；message-ui-thread.component.ts` |
| UI | 右侧回复抽屉/移动 sheet；root、总数、分页、composer；新回复同步主区最多三条预览 |

## Current / Target / Gap

- **Status**: open/load thread query = TARGET；replyMessage mutation 复用 G01 authority；live mapping = GAP。
- **Admission rule**: 类型、builder、fixture 或文档存在均不等于 CURRENT；focused readiness 只允许进入 017，runtime Green 只由最终真链证据建立。

## Gate-Specific Acceptance

- 本 Gate 只拥有 thread hydration/relation/page coverage；replyMessage task 仅做 binding 到既有 G01 replied command 的接线与回归，不复制发送状态机。
- 每个子链都必须覆盖 happy path、stale/cross-scope、authority reject、PersistErr、duplicate/late fact 和 baseRevision/epoch gap。

## Requirements

- **FR-001**: ActionBinding MUST 由 Helix capability 签发，平台只提交 actionRef 与声明 input。
- **FR-002**: Tick MUST 校验 actor、scope、epoch、revision、输入上限与 action lease。
- **FR-003**: 所有业务 mutation MUST 经权威事实和 PersistAtomic；中间 ack/log 不构成完成。
- **FR-004**: 所有受影响 Surface MUST 返回完整 typed VM/row 与单调 revision。
- **FR-005**: Frame MUST whole-frame validate/apply；失败则不部分提交并请求对应 snapshot。
- **FR-006**: Angular MUST 完成适用的 loading/submitting/ready/failed/forbidden/empty 与 PD 视觉状态。
- **FR-007**: 单 Gate 只跑 focused fixture/contract/component tests；Native、双账号、双库与视觉证据由 017 执行。

## Success Criteria

- **SC-001**: 相同 replay 输入产生字节等价 write-set 与 Frame。
- **SC-002**: 重复事实不推进 Frame；lookup O(1)/O(log n)，有界 compose O(visible)。
- **SC-003**: reachable Angular legacy/raw business reducer 为 0，成功态只由 matching Frame 建立。
- **SC-004**: Gate 达到 READY_FOR_FINAL_E2E，且没有为该 Gate 单独触发 Native 全量编译。

## Source

- `crates/helix-im/docs/interaction-v2/完整时序和数据/02-G08-G12-定时窗口定位与回复完整时序和数据.md §7`
- `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`
