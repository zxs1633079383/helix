# Feature Specification: G07 加急与确认

**Feature**: `007-g07-urgent` | **Created**: 2026-07-22 | **Status**: Draft | **Macro UC**: UC08

**Input**: 按 Gate 独立完成 加急与确认 的 Helix -> cses-client 全链路；单 Gate 不运行 Native 全量验收。

**Loop Profile**: `helix-loop/v1`

## User Scenario & Testing

### UC08 - 加急与确认 (P1)

**Trigger**: 发起人选择目标成员加急，目标成员确认，或有权用户取消。

**Independent Test**: 用 canonical Tick/权威 reply fixture 证明该 Gate 从 ActionBinding 到 PersistOk 后 Frame 的确定性结果；这只是 focused readiness，不是 Native/真实链 Green。

**Acceptance Scenarios**:

1. Given 合法 actor、attached surface 与当前 epoch/revision，When urgentMessage / confirmUrgent / cancelUrgent，Then 只产生受约束的 Tick/Effect，HTTP ack 不建立 UI 成功。
2. Given 权威事实到达，When UrgencyState(requester/targets/confirmed/revision) + viewer channel urgent + cursor 成功提交，Then 发布 matching-causation Frame，并由 Angular 机械渲染“闪电折角、发起/目标/已确认/观察者四态、确认按钮、会话头像角标与回执名单”。
3. Given 重复/乱序/失败/权限拒绝，When 处理完成，Then 不推进错误 revision、不泄漏其他 viewer 数据，保留上一版可渲染 VM。

## Loop Contract

- **Trigger**: 发起人选择目标成员加急，目标成员确认，或有权用户取消。
- **Goal**: 闪电折角、发起/目标/已确认/观察者四态、确认按钮、会话头像角标与回执名单。
- **Completion**: 权威事实、远端落库证据、本地 PersistOk、完整 Frame 与 PC DOM 同一 causation 对齐；最终证据由 017 一次性收集。
- **Boundaries**: 本 feature 不启动 Native E2E、不重复全量 cargo、不让 Angular 从 raw event/HTTP/多个 Surface join 业务事实。
- **Human Gate**: wire/权限语义与现网不一致，或需要改变 Go 业务语义时停止并确认。

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
| ActionBinding | `urgentMessage / confirmUrgent / cancelUrgent`；binding 带 actionRef、scope、epoch、revision、actor capability |
| Tick / Effect | `Tick::Command(ActionInvoked) -> urgentPost/urgentConfirm/urgentCancel HTTP Effect` |
| Authority | post_update + viewer update_channel；不同 viewer 独立事实 |
| Persist | `PersistAtomic(UrgencyState(requester/targets/confirmed/revision) + viewer channel urgent + cursor)`；只有 PersistOk 解锁业务 Frame |
| Surface | message.timeline、thread-detail、left-dialog-list、unread-summary、participant-receipts |
| VM / Row | 完整 MessageVm.urgency + DialogRowVm urgent badge + UnreadSummaryVm + ParticipantReceiptsVm |
| InteractionFrame | 同 viewer PersistOk 后多 Surface 原子 Frame；禁止泄漏完整目标名单 |
| Angular | `message-ui-urgent.component.ts；message-ui-read-receipt.component.ts；message-ui-sidebar.component.ts` |
| UI | 闪电折角、发起/目标/已确认/观察者四态、确认按钮、会话头像角标与回执名单 |

## Current / Target / Gap

- **Status**: urgentPost/confirm/cancel = TARGET；四 viewer 视角 live coverage = GAP。
- **Admission rule**: 类型、builder、fixture 或文档存在均不等于 CURRENT；focused readiness 只允许进入 017，runtime Green 只由最终真链证据建立。

## Gate-Specific Acceptance

- 分别测试发起、目标确认、发起者取消三种 binding；发起者、目标、非目标、无权限 viewer 的 capability、badge、detail 数据不可互相泄漏。
- 每个子链都必须覆盖 happy path、stale/cross-scope、authority reject、PersistErr、duplicate/late fact 和 baseRevision/epoch gap。

## Requirements

- **FR-001**: ActionBinding MUST 由 Helix capability 生成，Angular 只能回传 actionRef 与声明的 input。
- **FR-002**: Tick MUST 校验 actor/scope/epoch/revision/actionRef；无效输入零 HTTP、零 Persist、零业务 Frame。
- **FR-003**: mutation 成功 MUST 经权威事实和 PersistAtomic；HTTP 200/SUCCESS 不是业务完成。
- **FR-004**: 每个受影响 Surface MUST 下发完整 typed VM/row，不允许 partial JSON 或端侧 reducer。
- **FR-005**: Frame MUST 保持 surface/scope/baseRevision/revision/surfaceEpoch/causation 一致并原子 apply。
- **FR-006**: Angular MUST 使用列出的 Message-v4 组件完成 idle/loading/submitting/ready/failed/forbidden/empty 中适用状态。
- **FR-007**: focused fixture、contract 与 DOM component tests MUST 可单独失败；Native/双账号/双库证据推迟到 017。

## Success Criteria

- **SC-001**: canonical replay 相同输入字节产生相同 Persist write-set 与 Frame。
- **SC-002**: 重复权威事实产生 0 个新增业务 mutation；有界 compose 为 O(visible)，实体关联为 O(1) 或 O(log n)。
- **SC-003**: Angular 业务完成只由 matching Frame 建立，legacy raw event/HTTP response 业务 reducer 数量为 0。
- **SC-004**: 本 Gate 被标记 READY_FOR_FINAL_E2E，且未单独触发 Native 全量编译。

## Source

- `crates/helix-im/docs/interaction-v2/完整时序和数据/01-G04-G07-撤回转发回应与加急完整时序和数据.md §5`
- `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`
