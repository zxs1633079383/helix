# Feature Specification: G11 定位原消息

**Feature**: `011-g11-locate-message` | **Created**: 2026-07-22 | **Status**: Draft | **Macro UC**: UC11

**Input**: 按 Gate 独立完成 定位原消息 的 Helix -> cses-client 全链路；单 Gate 不运行 Native 全量验收。

**Loop Profile**: `helix-loop/v1`

## User Scenario & Testing

### UC11 - 定位原消息 (P1)

**Trigger**: 用户点击引用来源、搜索结果或通知中的 locate action。

**Independent Test**: canonical Tick/权威 reply fixture 必须独立证明 ActionBinding、Persist write-set 与 Frame；这是 focused readiness，不是 Native/真实链 Green。

**Acceptance Scenarios**:

1. Given 当前 actor/scope/epoch/revision，When locateMessage(channelId,targetMessageId)，Then 只生成合法 Tick/Effect，transport ack 不建立成功。
2. Given 本地完整 coverage 或校验后的 HTTP target/context 到达，When PersistAtomic 成功，Then 发布完整 matching Frame，PC 呈现“加载上下文后滚动到目标并短暂高亮；失败/无权限不改变当前窗口”。
3. Given 重复、乱序、超时、权限拒绝或 revision conflict，Then 不破坏上一版 VM，并给出可恢复的 rejected/failed 状态。

## Loop Contract

- **Trigger**: 用户点击引用来源、搜索结果或通知中的 locate action。
- **Goal**: 加载上下文后滚动到目标并短暂高亮；失败/无权限不改变当前窗口。
- **Completion**: 权威事实、远端 DB、本地 PersistOk、完整 Frame 与 PC DOM 因果一致；运行证据在 017 一次性收集。
- **Boundaries**: Angular 不消费 raw WS/HTTP、不 join 业务 Surface；本 feature 不启动 Native E2E。
- **Human Gate**: UiDirective 当前无 payload，禁止虚构 target；targetMessageId 必须在 TimelineWindowVm。

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
| ActionBinding | `locateMessage(channelId,targetMessageId)`，含 actionRef/scope/epoch/revision/capability |
| Tick / Effect | `Tick::Command(ActionInvoked) -> local-first lookup；miss 时 Effect::Http(/posts/get + /posts/postContext)` |
| Authority | 本地完整 coverage 或校验后的 HTTP target/context |
| Persist | `PersistAtomic(target + bounded context + coverage/index)`，PersistOk 前零业务 Frame |
| Surface | message.timeline |
| VM / Row | 完整 TimelineWindowVm(items,targetMessageId,page,state) 与 MessageVm rows |
| InteractionFrame | 先提交有序窗口，再用无 payload directive 请求平台 reveal；target 从 VM 读取 |
| Angular | `message-ui-surface-message-content.component.ts；message-surface-runtime-view.component.ts；history/message-anchor-index.ts` |
| UI | 加载上下文后滚动到目标并短暂高亮；失败/无权限不改变当前窗口 |

## Current / Target / Gap

- **Status**: locateMessage = TARGET；UiDirective payload = GAP/禁止。
- **Admission rule**: 类型、builder、fixture 或文档存在均不等于 CURRENT；focused readiness 只允许进入 017，runtime Green 只由最终真链证据建立。

## Gate-Specific Acceptance

- 测试 targetMessageId 存在于完整 TimelineWindowVm、Frame 成功 apply/ack 后 reveal、not-found 时无 directive，directive 本身仅有 directiveId/kind。
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

- `crates/helix-im/docs/interaction-v2/完整时序和数据/02-G08-G12-定时窗口定位与回复完整时序和数据.md §6`
- `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`
