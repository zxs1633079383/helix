# Source Discovery: InteractionFrame V2 双仓硬切

**Captured**: 2026-07-23 03:52 PDT

**Scope**: `/System/Volumes/Data/workspace/rust/helix` 与 `/System/Volumes/Data/workspace/angular/cses-client`

**Source Lock**: `helix=3b430c6cb1cd;cses-client=8d71085453c6;cses-im-server=6ddb52453641;native-helix-pin=154c7ba606629c402aa039f9c2c1e28cd0e2859c`

**Worktree State**: `source Helix/client/Go all contain pre-existing dirty or untracked files; implementation is isolated in codex/018-interaction-frame-v2-cutover worktrees`

**Rule**: 本文件记录 CURRENT；`spec.md` 与 `strict-matrix.md` 记录 TARGET/GAP。源码快照不是运行时 PASS。

## Facts *(mandatory)*

- **F-001**: Helix 通用 InteractionFrame schema 常量仍为 1，通用 builder 因此仍产生 V1。 | **Evidence**: `crates/helix-im/src/interaction_frame.rs:11`
- **F-002**: Helix V2 emitter 是与 V1 emitter 分离的独立入口，当前不能代表全部 Gate 已切换。 | **Evidence**: `crates/helix-im/src/projector/mod.rs:193`
- **F-003**: Angular Frame union 同时包含 InteractionFrameV1 与 InteractionFrameV2。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486`
- **F-004**: Angular codec 同时接受 V1/V2，并继续执行 legacy normalize。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:49`
- **F-005**: Angular V2 apply 在原子 swap 后确认 causation、执行 directive，并调用可选 `frameAppliedHandler`；全仓只有测试注册该 handler，因此不能声称生产 FrameApplied 已回传 Helix。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:222`
- **F-006**: Native 当前 Helix pin 与本次 Helix source lock 不同，最终候选尚未冻结。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/Cargo.toml:87`
- **F-007**: 旧 017 final Native 证据记录业务 Gate Green，但没有逐 Frame schema 计数。 | **Evidence**: `specs/017-uc04-16-native-e2e/evidence/final-native.md:1`

## Snapshot

- Helix 当前实现基线：`3b430c6cb1cd`。
- cses-client 当前 HEAD：`8d71085453c6`。
- cses-im-server 当前 HEAD：`6ddb52453641`。
- cses-client `src-tauri/Cargo.toml` 当前 Helix pin：`154c7ba606629c402aa039f9c2c1e28cd0e2859c`。
- cses-client 生成 Strict Matrix 声明的 Helix revision同为 `154c7ba606629c402aa039f9c2c1e28cd0e2859c`。
- Helix 实现基线与 Native pin 已明显分叉；当前生成合同仍跟随旧 pin，最终候选尚未冻结。
- Helix 源工作树存在 WS parser/test/G01 图等脏改动；cses-client 源工作树存在 message-v4 UI/SCSS 脏改动；Go 源工作树存在未跟踪文件。隔离 worktree 不包含这些未提交内容。

## Helix CURRENT

### F-H01 - 通用 Frame writer 仍是 V1

- `crates/helix-im/src/interaction_frame.rs:11` 定义 `INTERACTION_FRAME_SCHEMA_VERSION = 1`。
- `crates/helix-im/src/projector/mod.rs:161-170` 的通用 `build_interaction_frame` 使用该 V1 常量。
- `crates/helix-im/src/projector/mod.rs:178-190` 的通用 emitter 接受 V1 `InteractionFrame`。
- `crates/helix-im/src/query/local_first.rs`、`module/action.rs`、`surface_runtime.rs`、`dialog_frame.rs` 仍调用该通用 V1 builder/emitter。

结论：G01-G16 已经经过 Frame 边界，不等于已经使用 InteractionFrame V2。

### F-H02 - V2 writer 是独立恢复路径

- `crates/helix-im/src/projector/mod.rs:193-208` 提供独立 `emit_interaction_frame_v2`。
- `crates/helix-im/src/surface_composer.rs:64-122` 提供 `emit_recovery_v2_frame`。
- 当前调用点集中于 `sync/flow.rs` 与 `port_reply/dispatch_core.rs` 的恢复流程。

结论：GS 恢复已有 V2 producer，但不能据此推断所有 action/query/governance Gate 均为 V2。

### F-H03 - 当前 Strict Matrix 仍是 RenderNode/v1

- `crates/helix-im/contracts/strict-matrix.v1.json` 的 `render_protocol` 为 `RenderNode` version 1。
- 本 Feature 的 25 个机器 Gate 全部仍声明一个或多个 `*.v1` render kind，包括 `dialog.list.v1`、`message.left-dialog-list.v1`、`system.sync.v1`、`message.row.v1`、`channel.row.v1`、`member.row.v1`、`read.receipt.v1`、`reply.tree.v1`。
- 当前合同中目标 render kind 的出现次数至少为：`message.row.v1` 44、`channel.row.v1` 25、`member.row.v1` 16、`read.receipt.v1` 8、`reply.tree.v1` 6、`dialog.list.v1` 3；该计数覆盖全合同，不只本 Feature。

结论：当前机器合同不能证明 V2 SurfaceMutation 是唯一渲染协议。

### F-H04 - V2 目标文档已存在但仍标为 Draft/Planned

- `crates/helix-im/docs/interaction-v2/09-全Gate覆盖矩阵与raw-emit下线清单.md` 已定义 27 Gate 的 V2 Surface 目标。
- 该文档明确说明它是 Review Draft，不是 runtime green，并将多数 G02-G16c 标为 planned。

结论：目标设计可复用，运行状态必须重新由当前源码和证据判定。

## cses-client CURRENT

### F-C01 - V2 reader 能力存在，但仍是双读

- `surface-contract.types.ts:7-9` 同时定义 V1、V2 和 legacy schema 常量。
- `surface-contract.types.ts:486-503` 定义 `InteractionFrameV1 | InteractionFrameV2` union。
- `surface-contract.codec.ts:49-60` 同时接受 V1/V2，并对 V1 执行 normalize。
- `surface-runtime.service.ts:334-356` 只有 V2 才读取 runtime 和执行 directives；V1 仍能进入通用 mutation apply。

结论：客户端已经具备 V2 解码/apply 的基础，但尚未进入 fail-closed V2-only 状态。

### F-C02 - Angular V2 消费主链已经形成

当前公共链路为：

```text
im:interaction:frame
  -> ProjectionBusService
  -> decodeInteractionFrame
  -> ProjectionStore/SurfaceRuntime whole-frame apply
  -> MessageSurfaceFacade + 九 Surface adapters
  -> MessageComponent signals
  -> MessageSurfaceRuntimeViewComponent
  -> typed child components
```

源码证据：

- `projection-bus.service.ts` 订阅 `im:interaction:frame`。
- `surface-runtime/surface-contract.codec.ts` 是 raw-value 到 typed Frame 的唯一边界。
- `surface-runtime/surface-runtime.service.ts:300-363` 处理整帧 staging、revision gap、runtime、causation、directive 与 FrameApplied。
- `surface-runtime/message-surface.facade.ts:36-106` 暴露九个 typed slot。
- `message.component.html:11-60` 把 typed VM/status 交给 `message-surface-runtime-view`，并接收语义 UI output。

结论：Angular 的 V2 reader/Surface 结构可以作为迁移承接面，但 V1 分支和逐 Gate action 证明仍需清退/补齐。

### F-C03 - ActionBinding 入口存在，但逐 Gate 是否全部经 opaque binding 尚需证明

- `message-surface-action-bindings.component.ts:128-155` 按 actionRef 渲染并输出 `SurfaceActionRequested`。
- `actions/surface-action-invocation.ts:47-75` 校验 enabled、surface/scope、surfaceEpoch 与 observedRevision，只提交 actionRef/input。
- `shell.service.ts:31-37` 创建 action invocation draft。
- `command-port.ts:72-76` 使用 `im_invoke_action` 作为 SurfaceRuntime adapter。
- `message.component.html:33-60` 仍有多个语义 output/handler；它们是否最终全部使用 matching binding 必须逐 Gate 以代码和测试证明，不能仅由公共组件存在推断。

## Existing Evidence GAP

### F-E01 - 004-017 的 FULL GREEN 是业务基线，不是 V2 清退证据

- `specs/004-*` 至 `specs/016-*` handoff 均声明业务纵切 FULL GREEN。
- `specs/017-uc04-16-native-e2e/evidence/final-native.md` 记录 19 Gate、required fail=0、required skip=0。
- 该 final evidence 没有记录逐 Frame schemaVersion、V1 frame count、V2 frame count 或 V1 reader fallback count。
- 与此同时，当前 Helix 通用 writer 和 Strict Matrix 仍为 V1。

结论：旧证据可以防止 V2 迁移破坏业务，但不能满足本 Feature 的 `v1_frames=0` 完成条件。

## Impact Inventory

按当前受限模式扫描，至少涉及：

| 仓库 | 生产源码 | 合同/生成物 | 测试/fixture | 文档 | 合计 |
|---|---:|---:|---:|---:|---:|
| Helix | 9 | 2 | 8 | 3 | 22 |
| cses-client | 3 | 3 | 12 | 0 | 18 |
| 双仓 | 12 | 5 | 20 | 3 | 40 |

这是保守下界；逐 Gate action handler、组件测试和删除清单在实施计划阶段可能扩大，但不得因此扩大业务范围。

## Decisions Carried Into Spec

1. 先证明客户端 V2 reader 对合法/非法 V2 的完整行为，再迁移 writer。
2. writer 迁移必须覆盖 25 个机器 Gate，不能以 GS recovery V2 代表全量。
3. 更新 pin 前冻结四类 revision/hash，避免 HEAD、Cargo pin、生成合同和 fixture 漂移。
4. 首次全量 E2E 前必须切断 V1 producer/consumer；双读代码可以暂存，但生产运行时不得可达。
5. 首次纯 V2 E2E 必须显式统计并断言 V1 为 0；物理删除后必须生成新的回归证据，删除前 Green 不能签收删除后的 candidate。
6. `ActionInvocationEnvelope`、Strict Matrix 文档 schema 等独立版本不随 InteractionFrame 盲目升级。

## V1 Deletion Inventory

### InteractionFrame/RenderNode V1：首轮纯 V2 全绿后删除

| Repository | Category | Current reachable surface |
|---|---|---|
| Helix | producer | `INTERACTION_FRAME_SCHEMA_VERSION=1`、`build_interaction_frame`、`emit_interaction_frame` 及 module/query/dialog/surface runtime 调用点 |
| Helix | contract | `RenderNode/v1` 与 `*.row.v1` 在 Strict Matrix/command API payload 中的业务渲染声明 |
| Helix | fixture/test | `dialog_list_frame_v1.json` 与 `duplicate/gap/stale/snapshots/scope-mismatch/patch-transaction` schema 1 fixtures、V1 断言 |
| cses-client | consumer | `InteractionFrameV1` union、V1 codec 分支、`normalizeLegacyFrame` 与下游 legacy rich-field normalization |
| cses-client | fixture/test | mirrored schema 1 Frame fixtures、V1 apply/codec/projection-bus tests |
| cses-client | generated contract | generated `RenderNode/v1`、`*.row.v1` 业务映射与旧 reducer acceptance assumptions |

### 合法独立 `v1`：不得按名称误删

| Contract | Reason retained |
|---|---|
| `ActionInvocationEnvelope` schema 1 | 独立上行协议，不是 InteractionFrame V1 |
| `helix-im-command-api/v1` 文件/schema 标识 | 命令 API 文档版本；其中旧 render payload 要迁移，但 API 版本名不自动升级 |
| `helix-im-strict-matrix/v1` 文件/schema 标识 | Matrix 文档格式版本；其中 `RenderNode/v1` 业务声明要迁移 |
| E2E scenario、doctor、source-pin、evidence schema 的 `/v1` | 各工具自身版本，不是 UI Frame wire |
| `message.compose.text.v1` 等 input shape | Action input schema，除非另有协议变更不得跟随 Frame 盲改 |

### Raw fallback inventory

- Angular 生产 projection registry 已限定 renderable facts 进入 `im:interaction:frame`，不得重新订阅 raw post/channel/member 事件。
- 当前主要可达兜底是 Frame V1 decode/normalize，而不是独立 raw render reducer。
- evidence collector 尚未统计 `rawBusinessFallbacks`；在 T016 完成前只能记录“未观测”，不能记为零。
