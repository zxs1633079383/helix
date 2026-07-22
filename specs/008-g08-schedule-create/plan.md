# Implementation Plan: G08 创建定时消息

**Spec**: [spec.md](./spec.md) | **Macro UC**: UC09 | **Final Gate**: `../017-uc04-16-native-e2e/`

## Summary

纵切 G08：createSchedule(channelId,content,sendAt) 经 Tick/Effect、权威事实、PersistAtomic、SurfaceComposer 形成完整 Frame，PC 只 apply 与渲染。

## Architecture

- **领域归属**: helix-im 拥有权限、状态转换、幂等、持久 write-set、Surface impact 与 VM。
- **Application**: Module/ActionDispatch 编排 Tick -> Effect -> PortReply；不把 HTTP/DB/Angular 引入 domain。
- **Infrastructure**: host/native 实现 HTTP/WS/SQLite ports；Go server 提供远端事实；Tauri 只搬运 Frame。
- **Interface**: Angular SurfaceRuntime 校验 Frame、原子替换 cache、执行 directive；overlay/draft/route 留在平台。
- **模式**: 仅在变化点使用 Adapter(Go wire)、State(action ledger)、Repository(PersistAtomic)、Specification(capability/visibility)。

## Technical Contract

- **I-001**: post_schedule_created 或 owner-only getSchedule hydrate。
- **A-001**: createSchedule(channelId,content,sendAt) -> Tick::Command(ActionInvoked) -> Effect::Http(/api/cses/posts/createSchedule) -> authoritative fact -> PersistAtomic -> PersistOk -> Frame -> Angular render。
- **S-001**: idle -> accepted -> awaiting-authority -> persisting -> frame-ready；failed/stale/forbidden 回到上一业务 VM。
- **V-001**: focused Rust replay/contract test 证明 ActionBinding、Tick、write-set、Frame；不运行 Native build。
- **V-002**: focused Angular component/fixture test 证明 actionRef 回传、完整 VM 渲染和失败态；不连接真实服务。

## Loop Detail Contract

### Inputs and Context

- **I-101**: 当前 actor、attached Surface scope、surfaceEpoch/revision、host 签发 ActionBinding 或 HostObservation，以及本 Gate 声明的 canonical input。
- **I-102**: 经 Go HTTP query、WS 或 sync 到达且通过 audience/entity/revision 校验的权威输入；mutation 的 HTTP SUCCESS 只作为 request outcome。
- **I-103**: 当前持久事实、viewer index、cursor/eventSeq 与已 attach slot registry；不得读 DB 决定业务分支。

### Action Chain

- **A-101**: invoke/observation -> Tick -> capability/input 校验 -> canonical Effect；stale/cross-scope 时零 HTTP、零 Persist、零业务 Frame。
- **A-102**: authority -> normalize -> 单批 PersistAtomic -> matching PersistOk -> SurfaceComposer -> 完整 InteractionFrame -> PC whole-frame apply。
- **A-103**: PersistErr/authority reject 保留上一致 VM；duplicate/late fact 幂等；baseRevision/epoch gap 整帧拒绝并 resync。

### State Contract

- **S-101**: `Idle -> Accepted -> AwaitingAuthority -> Persisting -> FrameReady`；查询/observation Gate 可跳过 outbound mutation，但不能跳过 PersistOk。
- **S-102**: `Rejected | RecoverableFailure` 不改变业务 projection；detached slot 只持久化，下一次 attach 从事实生成 snapshot。
- **S-103**: 同一 entity/viewer revision 的重复输入为 self-loop，不产生第二个业务 mutation。

### Validators

- **V-101**: Rust red-green replay/contract 验证 action/Tick/authority/write-set/PersistOk/Frame 与 Gate 特有分支。
- **V-102**: Angular red-green component/DOM 验证 typed invoke、whole-frame apply、业务状态、ARIA/data-* 和无 raw reducer。
- **V-103**: loop R1-R3 与 speckit-analyze 验证治理、追踪、tests-first、worktree 与 requirement coverage。
- **V-104**: 017 final candidate 才验证双账号、Go/PG、SQLite、Frame、DOM 与 Index-Dual；本 Gate 不单独 Native build。


## Constitution Check

*GATE: 计划形成后通过；实现准入仍以 R1-R3 当前指纹 PASS 为准。*

- **I Runtime-Agnostic Layering - PASS**: 领域和状态机仅在 `helix-im`；HTTP/WS/SQLite/Tauri/Angular 只实现 port 或协议转换。
- **II Deterministic Facts/Effects/Persistence - PASS**: 时间、identity、authority 均由 host/Go 输入；mutation 只在匹配 `PersistOk` 后可见。
- **III InteractionFrame V2 - PASS**: 平台只提交 binding input 并 whole-frame apply；完整 VM 是唯一渲染真源。
- **IV Evidence Before Completion - PASS**: focused Rust/Angular 只建立 readiness；双账号、PG、SQLite、Frame、DOM、visual 由 017 最终候选统一证明。
- **V Minimal Governed Migration - PASS**: 复用现有 Effect、StorageOp、SurfaceRuntime 与 Message-v4 组件；不新建并行 raw reducer。
- **VI Spec Kit Loop - PASS**: discovery/data-model/contract/tasks/verification 已具备；R1-R3 由 `loop_engine.py` 独立生成 ledger。
- **Safety - PASS**: actionRef actor/scope 绑定；日志脱敏；DB 只读验收；同一失败签名最多两次自动修复。

## Repository Isolation

- Helix owner: `/System/Volumes/Data/workspace/rust/helix`，实施前记录 HEAD/branch/dirty state；只改本 Gate 明确文件。
- Go owner: `/System/Volumes/Data/workspace/golang/cses-im-server`，仅 wire/authority 根因需要修改时创建独立 worktree；不与 Helix worktree 混用。
- PC owner: `/System/Volumes/Data/workspace/angular/cses-client`，记录 consumer pin 与既有修改；Angular/Tauri 修改使用独立 worktree 或在原 owner 仓最小协作。
- 跨仓 contract 变更按 producer -> fixture/schema -> consumer pin 顺序提交；单 Gate 不运行 Native build。

## Gate-Specific Plan

- **create plus hydrate**: 创建等待 WS；getSchedule owner-only query 负责 attach hydration 并走 PersistOk。
- 红灯 fixture/contract/component test 必须先落盘并证明可失败，之后才允许生产实现；focused 验证完成后只登记 `READY_FOR_FINAL_E2E`。

## Change Surface

### Helix

- `crates/helix-im/src/projector/action.rs`
- `crates/helix-im/src/interaction_frame.rs`
- `crates/helix-im/src/surface_composer.rs`
- `crates/helix-im/src/outbound/posts/existing.rs`
- `crates/helix-im/src/ws/handlers/post_schedule_created.rs`
- `crates/helix-im/src/projector/composer.rs`
- `crates/helix-im/src/dialog_frame.rs`
- `crates/helix-im/tests/` 与 `crates/helix-im/contracts/` 的本 Gate fixture/schema

### cses-client

- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-schedule-dialog.component.ts`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-composer.component.ts`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/components/message-ui-surface-dialog-list.component.ts`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.scss`

## Complexity & Observability

- actionRef ledger 与实体 lookup O(1)；有序页/窗口 O(visible) 或 O(n log n) 显式排序一次；禁止模板 find/filter/sort 形成 O(n²)。
- 每跳保留 corr_key/causationId：ActionInvoked、Tick::Command、HTTP/WS/sync、Persist request/reply、Frame、apply/ack。
- 单 Gate 只记录 focused evidence 和 READY_FOR_FINAL_E2E；Go/client 日志仅在 017 失败时按相关键读取。

## Verification Cadence

1. 写失败 fixture/contract。
2. 完成 Rust 与 Angular 纵切，中途禁跑 cargo。
3. 批量运行本 Gate focused tests；修复时一次性读完失败再批量改。
4. 不运行 Native E2E；把最终场景、observer 差异和 DB 断言登记给 017。
