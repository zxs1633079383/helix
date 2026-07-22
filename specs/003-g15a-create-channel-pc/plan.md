# Implementation Plan: G15a PC 创建群聊

**Branch**: `main (spec-only; no branch hook configured)` | **Date**: 2026-07-21 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/003-g15a-create-channel-pc/spec.md`

## Summary

在 `DialogListVm` 根级发布稳定的 `channel.create` ActionBinding，由 PC overlay 只负责群名称和成员选择；提交后通过 `Tick::Command` 进入 `helix-im`，复用现有 `channel/create` HTTP 合同，并把 HTTP、WS 与恢复同步取得的权威频道事实统一收敛为一次 `PersistAtomic(Channel + ChannelMember + ViewerChannelState)`。只有关联的 `PersistOk` 才能更新会话列表 Frame 和结束本次 action。移动端、Web 专属 UI、话题和频道治理不在本计划内。

## Technical Context

**Language/Version**: Rust workspace toolchain；Angular 20 + TypeScript（PC `cses-client`）

**Primary Dependencies**: `helix-im`、`helix-core` 的 `Effect`/`PortReply`/InteractionFrame ports、现有 `channel/create` outbound adapter、Angular message-v4 SurfaceRuntime

**Storage**: 复用 Helix SQLite 的 channel/member/viewer state 操作与现役 Go 服务端 PostgreSQL 权威事实；不新增 schema，不直接写库

**Testing**: Rust deterministic replay/contract tests，Angular Jest component/DOM tests，Spec Kit R1-R3 validators；真 Go 双用户链路作为实现后的独立验收

**Target Platform**: PC Tauri；共享 `helix-im` 语义保持 runtime-agnostic，移动端与 Web 专属 UI 不验收

**Project Type**: 跨仓库业务模块 + 桌面客户端

**Performance Goals**: 成员归一化与成员存储为 `O(m)`；Frame 合成为挂载窗口内的 `O(k)` mutation/有界列表更新；重复事实不增加第二行或第二份成员关系

**Constraints**: `helix-core` 零业务/平台依赖；无 HTTP 成功推断；成功 Frame 必须晚于 `PersistOk`；ActionBinding actor/scope/epoch/revision fail-closed；外部输入零 `unwrap`/`expect`；跨边界批处理；不引入移动端任务

**Scale/Scope**: 一个 G15a capability、三条权威事实入口（HTTP reply、WS、恢复 sync）、一个 PC overlay、创建者/受邀者/未参与者三种用户范围

## Constitution Check

*GATE: Phase 0 前与 Phase 1 后均通过。*

- **I Runtime-Agnostic Layering — PASS**: 新业务语义只进入 `helix-im`；PC overlay 在 `cses-client`；不改 `helix-core` 端口或引入平台依赖。
- **II Deterministic Facts/Effects/Persistence — PASS**: HTTP、WS、sync 只提供权威输入；统一用关联的 `PersistAtomic`，成功可见性只由 `PersistOk` 推进。
- **III InteractionFrame V2 — PASS**: capability 位于 `DialogListVm.actions`；PC 只保留 draft/overlay，本地不重算频道、成员、权限或未读业务事实。
- **IV Evidence Before Completion — PASS**: Rust 证明 effect/persist/frame 顺序，Jest 证明 PC binding/overlay/DOM，真链证明远端事实与双用户隔离；任一缺失均不得宣称完成。
- **V Minimal Governed Migration — PASS**: 复用现有 command、storage op、Frame patch 和 action invocation；仅补 dialog 根级 action 与建群关联状态，不新建并行协议。
- **VI Spec Kit Loop Admission — PASS**: `discovery.md`、R1 ledger、plan/data model/contract/quickstart 已形成；R2/R3 由 hook 独立验证后才能实施。
- **Safety — PASS**: 成员/输入集合受现有 SurfaceData 上限约束；单批持久化；无直接 SQL 修复、无 raw reducer fallback、无移动端扩面。

**Phase 1 re-check**: `research.md` 的六项决策、`data-model.md` 的状态机和 `contracts/g15a-create-channel-interaction.md` 的 wire/Frame 顺序均未引入例外；无需 Complexity Tracking 豁免。

## Loop Detail Contract

> 外置 UC 合同见 `spec.md`；本节只定义实现可消费的输入、动作、状态与验证标识。

### Inputs and Context

- **I-001**: 已挂载的 `message.left-dialog-list@User` Frame、当前 actor/scope/epoch/revision 与根级 `channel.create` ActionBinding；仅允许读 action invocation 和当前 surface state。来源：F-001、F-002。
- **I-002**: PC overlay 产生的规范化草稿 `{display_name, user_ids}`；名称 trim 后 1..30 字符，成员去空、去重且排除创建者，至少一名受邀者。overlay 中间态不得进入共享业务状态。来源：F-003、F-004。
- **I-003**: 现有 outbound canonical request `{type, user_ids, display_name, force_create}` 与 host 注入的 request/correlation identity；不得接收 creator/team/role/avatar 等服务端派生字段。来源：F-005、F-006。
- **I-004**: HTTP reply、`channel_created` WS 或恢复 sync 提供的权威 channel/member fact，以及关联的 storage PortReply。不得读取数据库来决定业务分支。来源：F-007、F-008、R-003。

### Action Chain

- **A-001**: `DialogListVm.actions[channel.create]` + 合法 invocation -> `Tick::Command(CreateChannel)` -> 严格校验 actor/scope/epoch/revision/input -> 一个 canonical HTTP Effect；同一 action/request 在 in-flight 时拒绝重复发送。
- **A-002**: 合法 HTTP/WS/sync 频道事实 -> 共享 `ChannelCreatedCommit` 归一化 -> 一个 `Effect::PersistAtomic` 批次写 channel、唯一 members 与 viewer state；此时不得发布成功 Frame。
- **A-003**: 匹配的 `PersistOk` -> 幂等更新当前用户的 DialogList frame state -> 原子 Frame mutation（创建者带 causation id）+ action terminal receipt；未挂载时只保留可恢复事实。
- **A-004**: 输入、能力、HTTP、权威结果或持久化失败 -> 保持上一致 Frame，不插入频道行 -> 关联失败 receipt/directive；PC overlay 保留 draft 并解除 submitting。
- **A-005**: 创建者进入新频道 -> 复用 GS04 取得同频道 summary + composer bundle -> bundle 可用后才允许发送；timeline hydration 独立进行。

### State Contract

- **S-001**: `Drafting` -> `Submitting(request_id)` 仅在 binding 与输入均有效时允许；非法输入停留 `Drafting`。
- **S-002**: `Submitting(request_id)` -> `AwaitingAuthoritativeFact(request_id)` 在 canonical HTTP Effect 被接受后发生；重复 invocation 不产生第二个 Effect。
- **S-003**: `AwaitingAuthoritativeFact`/`ObservedAuthoritativeFact` -> `Persisting(corr, channel_id)` 只通过共享事实归一化器发生；HTTP/WS/sync 重复输入合并到同一 channel key。
- **S-004**: `Persisting` -> `Visible(channel_id)` 仅由匹配 `PersistOk` 发生；随后一次性发布当前 user scope Frame。`PersistErr` -> `RecoverableFailure`，不得可见。
- **S-005**: `Visible` + 重复权威事实 -> `Visible` 幂等自环；成员关系与会话行均保持唯一。
- **S-006**: 未挂载 surface 收到并持久化事实 -> `PersistedDetached`；下次挂载从持久事实生成 `Visible`，不重放 create HTTP。

### Validators

- **V-001**: Rust action contract tests 证明根级 binding、严格输入、单 HTTP Effect 与 stale/cross-scope fail-closed，覆盖 FR-001..FR-005。
- **V-002**: Rust replay tests 断言 `PersistAtomic` 前后 effect 顺序，并以“删除 PersistOk 则测试失败”证明成功 Frame 只能晚于持久化，覆盖 FR-006..FR-008、FR-012。
- **V-003**: Rust duplicate/order/recovery tests 证明 HTTP/WS/sync 任意顺序只形成一个 channel、唯一 members 与每用户一个 frame result，覆盖 FR-010、FR-011、FR-013。
- **V-004**: Angular Jest/DOM tests 证明 PC 从 typed binding 打开 overlay、合法提交携带原 binding、失败保留 draft、成功只消费 Frame；不得调用旧 direct create reducer。
- **V-005**: 只读双用户真链检查关联同一 request/channel 的远端 channel/member、两份独立用户 scope Frame 及未参与用户零变化；命令、起止时间与耗时写入 evidence。
- **V-006**: Spec Kit analyze + loop R2/R3/R4 validator 证明 spec/plan/tasks/verification 可追踪且移动端任务为零。

## Project Structure

### Documentation (this feature)

```text
specs/003-g15a-create-channel-pc/
├── discovery.md
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── verification.md
├── review-ledger.json
├── contracts/
│   └── g15a-create-channel-interaction.md
├── evidence/
└── tasks.md
```

### Source Code (repository roots)

```text
/System/Volumes/Data/workspace/rust/helix/
├── crates/helix-im/src/
│   ├── interaction_frame.rs
│   ├── dialog_frame.rs
│   ├── module/action.rs
│   ├── projector/action.rs
│   ├── port_reply/
│   └── ws/handlers/
├── crates/helix-im/tests/
└── crates/helix-im/contracts/

/System/Volumes/Data/workspace/angular/cses-client/
└── src/pages/message-v4/
    ├── components/message-ui-create-chat.component.ts
    ├── surface-runtime/
    ├── shell.service.ts
    └── actions/surface-action-invocation.ts
```

**Structure Decision**: 领域与 application 编排留在 `helix-im`，现有 Effect/Storage ports 承担 infrastructure 边界，PC 仅在 message-v4 SurfaceRuntime 中承载 overlay 和 Frame apply。没有新增 core crate、driver、后端接口或移动端目录。

## Complexity Tracking

无 Constitution 例外；不新增模式或并行状态真源。
