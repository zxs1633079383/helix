# Feature Specification: G16/G16b/G16c 频道关闭、退出与治理

**Feature**: `016-g16-channel-governance` | **Created**: 2026-07-22 | **Status**: Draft | **Macro UC**: UC16

**Input**: 按 Gate 独立完成 频道关闭、退出与治理 的 Helix -> cses-client 全链路；单 Gate 不运行 Native 全量验收。

**Loop Profile**: `helix-loop/v1`

## User Scenario & Testing

### UC16 - 频道关闭、退出与治理 (P1)

**Trigger**: 有权用户 close/remove/change settings，或当前用户 leave。

**Independent Test**: canonical Tick/权威 reply fixture 必须独立证明 ActionBinding、Persist write-set 与 Frame；这是 focused readiness，不是 Native/真实链 Green。

**Acceptance Scenarios**:

1. Given 当前 actor/scope/epoch/revision，When closeChannel / leaveChannel / changeMembers / changeRole / changeSettings，Then 只生成合法 Tick/Effect，transport ack 不建立成功。
2. Given channel_close、channel_member_update、update_channel/notice/role/nickname/approval events 到达，When PersistAtomic 成功，Then 发布完整 matching Frame，PC 呈现“关闭后只读、自退群退出当前页面、移除仅影响目标成员、设置按版本刷新；失败保留表单与旧 Frame”。
3. Given 重复、乱序、超时、权限拒绝或 revision conflict，Then 不破坏上一版 VM，并给出可恢复的 rejected/failed 状态。

## Loop Contract

- **Trigger**: 有权用户 close/remove/change settings，或当前用户 leave。
- **Goal**: 关闭后只读、自退群退出当前页面、移除仅影响目标成员、设置按版本刷新；失败保留表单与旧 Frame。
- **Completion**: 权威事实、远端 DB、本地 PersistOk、完整 Frame 与 PC DOM 因果一致；运行证据在 017 一次性收集。
- **Boundaries**: Angular 不消费 raw WS/HTTP、不 join 业务 Surface；本 feature 不启动 Native E2E。
- **Human Gate**: close、自退群、管理员移除不得复用同一 ActionBinding；settingsVersion 冲突必须重新 hydrate。

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
| ActionBinding | `closeChannel / leaveChannel / changeMembers / changeRole / changeSettings`，含 actionRef/scope/epoch/revision/capability |
| Tick / Effect | `Tick::Command(ActionInvoked) -> /channel/close、/channel/member/leave|change、/channel/change/* Effects` |
| Authority | channel_close、channel_member_update、update_channel/notice/role/nickname/approval events |
| Persist | `PersistAtomic(Channel lifecycle/settingsVersion + memberships/roles + viewer indexes + system message + cursor)`，PersistOk 前零业务 Frame |
| Surface | left-dialog-list、unread-summary、channel-summary、member-roster、topic-list、timeline、composer |
| VM / Row | 完整 DialogRowVm/ChannelSummaryVm/MemberRowVm/TopicRowVm/System MessageVm/ComposerVm |
| InteractionFrame | close/leave/remove/settings 是不同 terminal patch；viewer-specific fallback/dismiss directive 只在 Frame apply 后 |
| Angular | `message-ui-group-settings-drawer.component.ts；message-ui-member-drawer.component.ts；message-ui-channel-management.component.ts；message-ui-confirm-dialog.component.ts` |
| UI | 关闭后只读、自退群退出当前页面、移除仅影响目标成员、设置按版本刷新；失败保留表单与旧 Frame |

## Current / Target / Gap

- **Status**: close/leave/member.update/channel.update = 各自 TARGET；live mapping = GAP。
- **Admission rule**: 类型、builder、fixture 或文档存在均不等于 CURRENT；focused readiness 只允许进入 017，runtime Green 只由最终真链证据建立。

## Gate-Specific Acceptance

- close、self leave、remove member、role change、settings change 五条子链分别测试 binding、wire、authority、viewer impact 和 terminal Frame；禁止用一个泛化 change test 代替。
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

- `crates/helix-im/docs/interaction-v2/完整时序和数据/04-G15-G16c-建群话题与频道治理完整时序和数据.md §4-§6`
- `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`
