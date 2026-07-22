# Implementation Plan: SPEC-IFV2-G01-在线发送

**Branch**: `002-ifv2-g01-online-send` | **Date**: 2026-07-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-ifv2-g01-online-send/spec.md`

## Summary

普通文本发送改为唯一的 `InteractionFrame v2` 下行链：PC 从
`message.composer@Channel` 读取签名 `ActionBinding`，提交版本化动作信封；`helix-im`
校验后以确定性 Tick 产出 `PersistAtomic`。只有临时事实 `PersistOk` 后，
`SurfaceComposer` 才生成含 `Sending` Row、受影响 dialog Row 和 runtime 的完整帧。
HTTP 2xx 仅记录 transport accepted；只有 WS echo 或权威同步持久化成功后才有 `Sent`。
客户端验证整帧、copy-on-write 原子替换 Frame Cache，发出 `FrameApplied`，再执行
`ClearComposerDraft` directive。失败、重复和迟到确认同样经事实持久化和帧收敛，
不回落到 raw emit 或客户端 reducer。

这是 G01 的首批纵切。它明确九个 Surface 中被 G01 影响的部分，但不实现 G02-G16c。
GS01-GS04 同步恢复由 `specs/001-gs-sync-recovery/` 独立计划；两者共享 V2 wire、
Frame Cache 与恢复语义。

### GS01-GS04 admission

`001-gs-sync-recovery` 是本计划的硬前置，不是可并行补齐的参考资料。只有 001 的
T050 在**当前 source pin**记录 Rust、PC DOM、mobile fixture、SQLite/PG readonly 四面
明确 `PASS` 后，002 才能创建/使用实现 worktree、运行 Cargo/Jest 或执行任何 G01 源码任务。
在此之前，002 仅维护文档和执行只读审计。九 Surface 定义、公共 camelCase wire、Frame
Cache 原子算法、通用 fixture grammar、hydration 后的 summary+composer bundle 均由 001
交付；G01 只在该已验收基线上扩展文本 content、delivery、`im.send-message` action 和
受影响 slot 的 composition，绝不重复定义这些共享能力。

## Technical Context

**Language/Version**: Rust workspace（`helix-core` / `helix-im`），Angular 20 + TypeScript（`cses-client`）；以当前 `Cargo.toml`、`package.json` 锁定版本为准。

**Primary Dependencies**: `helix-core` Tick/Effect/Port；`helix-im` send、action、projector、InteractionFrame；Angular `ProjectionBusService`、`SurfaceRuntimeService`、`ProjectionStoreService`、message-v4 adapters；Tauri native ingress。

**Storage**: Helix driver 的本地 SQLite 为产品持久化端口；远端 Go/PostgreSQL 是权威服务端记录。验证只能用 `helix_local_sqlite_ro` 与 `cses_pre_postgres_ro` 的 bounded `SELECT`；禁止 direct SQL repair/write。

**Testing**: Rust `cargo test -p helix-im` 行为/合同/fixture/replay；`cses-client` Jest/Angular DOM；双用户真实链；SQLite/PG 只读对账。

**Target Platform**: native PC/Tauri 为本期完成前置；Mobile/Web 复用 generated V2 wire/Frame Cache 语义，非本 Spec UI 完成前置。

**Project Type**: Runtime-agnostic Rust 业务内核 + 桌面客户端渲染集成。

**Performance Goals**: 每帧 Frame Cache `O(k)` 原子 apply，`k` 为 mutation 数；每个 window replace 受既有窗口上界约束；禁止全量 raw-event replay、跨 Surface 业务 join 与无界 JSON 拷贝。

**Constraints**: core 不认识 IM/UI schema，不依赖 runtime/SQLite/HTTP；`helix-im` 仅以 Effect 请求 IO；Persist 是 Sending/Sent 可见性的硬屏障；unknown schema/surface/epoch/revision/content kind fail-closed；HTTP 2xx 不等于 Sent；不得 dual-write raw emit；不得新增发送上限或将 actionRef 变为全局单飞锁；001 T050 四面 `PASS` 前不得创建 G01 实现 worktree、运行实现测试或改业务源码；公共 JSON wire 只用 camelCase，Rust snake_case 仅可作为 serde 内部表示。

**Scale/Scope**: 仅普通文本 G01。直接影响 `message.composer`、`message.timeline`、`message.left-dialog-list`，条件影响 `message.thread-detail`；`message.unread-summary`、`message.channel-summary`、`message.member-roster`、`message.topic-list`、`message.participant-receipts` 保持合同兼容，本纵切不凭空产生 mutation。

## Constitution Check

本计划受 `.specify/memory/constitution.md` 的 Helix Constitution 约束；它把仓库
`AGENTS.md`、`docs/GOAL.md`、`docs/ROADMAP.md`、InteractionFrame V2 分册合同落为
可执行门禁。001 T050 准入通过后、启动首个 G01 实现任务前必须再次核对。

| Gate | 设计结论 | 状态 |
|---|---|---|
| 三层依赖 | `helix-core` 不新增 IM/UI/SQLite/HTTP 依赖；G01 语义在 `helix-im`，driver/PC 是 adapter。 | PASS |
| 确定性与 I/O | 输入是 action / port reply / inbound / timer Tick；ID、时间、IO 均由 host 注入。 | PASS |
| Persist 屏障 | `Sending` / `Sent` Frame 分别只在对应 `PersistOk` 后输出。 | PASS |
| V2 唯一真源 | 新 G01 V2 flow 只产生 InteractionFrame V2，PC 无 raw reducer fallback；legacy raw 删除遵循已批准 inventory 的单次物理删除。 | PASS，T021 准备、T039 审查、T040 删除、T041 取证 |
| 原子消费 | PC Cache copy-on-write 验证整帧后提交；directive 位于 FrameApplied 后。 | PASS |
| 安全可审计 | binding 校验 actor/scope/epoch/revision；MCP 仅只读、范围受限、以 `corrKey` 串联。 | PASS |
| 观察性 | action、tick、effect、persist、frame、cache、ack/directive 必须带同一 `corrKey`/`requestId`。 | PASS，实施补 hop 测试/日志 |
| 001 基线准入 | 002 只消费 001 已完成的 V2 shared baseline；以 T050 的当前 pin 四面证据解锁实现。 | BLOCKED，等待 001 T050；此前仅文档/只读审计 |

**Post-design re-check**: `research.md`、`data-model.md`、`contracts/interaction-frame-v2-g01.md` 没有把业务状态推入 core 或平台，也未引入 direct SQL、raw fallback、HTTP-success-to-Sent 快捷路径；门禁继续 PASS。

## Architecture And Execution Strategy

### Ownership and boundaries

0. **001 shared baseline**: hydration committed 后，001 才以同一 bundle 发出 channel summary、composer 和 runtime；它授予 `message.create` capability。其 opaque binding 的固定 action kind 是 `im.send-message`，输入合同是 `im.message-content.v2`。002 只验证和消费此 binding，不能从 channel availability、路由或组件自行签发。
1. **PC overlay / host**: 编辑器保存本地 draft、光标、动画。回车可立即清可见文本，但按 `requestId` 保存 recovery draft；它只从 composer VM binding 构造 `ActionInvocationEnvelopeV2`，不得自造 action kind、channel/user/temporary ID 或插入消息 row。
2. **`helix-im` application/domain**: dispatch 校验 binding，登记 pending send，安排 persist/send/timer/reconcile。storage、HTTP、WS 仍经 Effect/PortReply；`SurfaceComposer` 从 committed fact set 产出 G01 typed content/Row、binding、frame，而不重写 001 的 shared surface/cache 合同。
3. **driver**: 原子执行 temporary message + pending relation 或 canonical settle 写集，返回带 correlation 的 `PersistOk/Err`；HTTP/WS 结果回灌为 port/inbound Tick。
4. **Frame Cache**: PC/Mobile 沿用 001 的 V2 decode、全帧验证和 copy-on-write 原子提交；002 只增加 G01 typed content/delivery decode 与 G01 directive 使用，随后 ack `FrameApplied`，再尝试 directive 并 ack `DirectiveApplied`。

### G01 state machine

```text
DraftOverlay
  --valid ActionBinding / Command Tick--> PersistingTemporary
  --PersistOk--> Sending + Frame(sending) + Http(CreatePost) + timeout
  --Http 2xx--> AwaitingCanonical                 (presentation remains Sending)
  --WS echo or sync + PersistOk--> Sent + Frame(sent)
  --timeout/remote failure + PersistOk--> Failed + Frame(failed)

Rejected / PersistErr before Sending Frame -> recovery draft; no Row
duplicate / late canonical -> idempotent settle; one logical Row only
```

`ActionReceipt` 只改变 overlay 的 accepted/rejected 反馈。matching Frame 是 renderer 唯一的
业务状态更新；这隔离暂态 UX 与持久业务事实。

### Affected Surface composition

| Committed fact | Required slots | Conditional slots | Explicitly unchanged |
|---|---|---|---|
| Temporary persisted as Sending | `message.timeline@Channel`, `message.left-dialog-list@Viewer` | `message.thread-detail@Channel/root` for attached reply | composer capability unless policy changes; unread/roster/topic/receipt/channel-summary |
| Canonicalized as Sent | same timeline + dialog | same thread | same unchanged set |
| Failed after Sending | timeline, dialog only when preview fact changes | thread if attached reply | no receiver/unread temporary mutation |
| Receiver canonical persist | receiver dialog/timeline/thread when attached; unread only if fact changes | none | sender temporary state never crosses viewer |

composer owns capability；timeline owns delivery row；dialog owns preview/sort。新消息种类扩展
`VersionedMessageContent` 和 `im.message-content.v2` codec registry，不增长 composer 根字段。
binding 与所属 Row/VM 同帧更新，避免旧权限地址悬挂。

### Hard-cut migration

1. 先核验 001 T050 的当前 pin 四面 `PASS`，再消费已固化的 V2 wire/fixture evidence；producer 与 V4 consumer 作为同一行为变更迁移。
2. T021 建立新 G01 flow 的 `InteractionFrame v2` **唯一**业务下行；它没有 raw mirror/fallback，并产出待删除 legacy raw inventory，但不在此任务物理删除既有 producer。
3. T039 先复核 001 T050 证据、G01 + GS01-GS04 的 V2 文档/27 Gate，针对 inventory 记录明确的 approve/reject 决定；未获批准即停止在 V2-ready，不能声称 raw 已下线。
4. 仅 T040 在 T039 批准后物理删除 legacy G01 producer、obsolete fixture expectations、dead subscriptions；T041 对已删除状态运行 grep、Rust/PC/DOM 和证据矩阵。不得由此声称 G02-G16c 已具备下行；它们各自需要 V2 composer/frame 实现。

### Evidence and progress protocol

每个运行固定 `corrKey`/`requestId`、两个隔离账号、可清理频道。证据包包含：accepted action、
Tick/Effect/Persist hop、Sending/Sent/Failed frame decoded summary、cache apply/ack/directive、
SQLite/PG 只读结果、sender/receiver DOM、至少一个反证。任一证据缺失即 `blocked`/`inconclusive`。
工作超过 30 分钟时，每 30 分钟报告完成项、当前证据、阻塞和下一步；这是可见状态 cadence，不启动 cron。

## Project Structure

### Documentation (this feature)

```text
specs/002-ifv2-g01-online-send/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/interaction-frame-v2-g01.md
└── tasks.md
```

### Implementation worktrees

本次执行绝不写入任一 dirty primary checkout。下列路径只是**预留别名**；在 T001 已核验
001 T050 当前 pin 四面 `PASS` 前，不得创建或使用它们：

- `HELIX_WORKTREE=/System/Volumes/Data/workspace/rust/helix/worktrees/ifv2-g01-online-send-20260721`
- `CSES_CLIENT_WORKTREE=/System/Volumes/Data/workspace/angular/cses-client/worktrees/ifv2-g01-online-send-20260721`

下方和 `tasks.md` 中的 `CSES_CLIENT_WORKTREE` 是第二个根的显式别名。Mobile/Web
只消费同一 V2 wire 语义，不是本 G01 Spec 的代码完成前置，因而不建立空 worktree 或
把没有任务的端侧改动伪装为本次交付。

### Source Code

```text
crates/helix-im/
├── src/
│   ├── interaction_frame.rs
│   ├── module/action.rs
│   ├── projector/action.rs
│   ├── send/{flow.rs,pending.rs,reconcile.rs}
│   └── surface_composer.rs
└── tests/
    ├── send_test.rs
    ├── f1_send_http_reconcile_test.rs
    ├── interaction_frame_contract_test.rs
    ├── f5_golden_replay.rs
    └── fixtures/uc01-16-surface-runtime/

CSES_CLIENT_WORKTREE/src/pages/message-v4/
├── projection-bus.service.ts
├── projection-store.service.ts
├── surface-runtime/{surface-contract.types.ts,surface-contract.codec.ts,surface-runtime.service.ts,message-surface.facade.ts,timeline.adapter.ts}
└── components/message-surface-runtime-view.component.ts
```

**Structure Decision**: 不新建跨层 shared UI store。001 拥有 shared V2 wire、fixture、Frame Cache 与 hydration/composer 基线；`helix-im` 在其上拥有 G01 typed interaction semantics；PC service 只做 mechanical cache/apply/ack 和 G01 typed codec，component 只读 VM。driver/storage/HTTP 不搬进 core/Angular。

## Implementation Phases

0. **GS admission (documentation/read-only)**: 读取 001 T050，核验同一 source pin 的 Rust、PC DOM、mobile fixture、SQLite/PG readonly 四面 evidence；未通过则保持所有实现任务 blocked，不创建 worktree、不跑实现命令。
1. **G01 contract/test extension**: 在已验收 V2 baseline 上增加 G01 discriminated text content、causation、delivery transition 和 G01 golden fixture；先覆盖 persist barrier、HTTP 2xx、duplicate/late echo、bad frame atomic rejection，不重新定义 Frame Cache 或九 Surface grammar。
2. **Helix producer**: valid accepted binding -> Tick -> pending -> `PersistAtomic` -> `PersistOk` composer；输出 Sending/Sent/Failed typed Row 及 affected slots；保留 receiver isolation；新 flow 不 dual-write raw，遗留 raw 的物理删除仅在 T039 审查批准后由 T040 执行。
3. **PC V4 consumer overlay**: 使用 001 的 generated V2 codec/Cache，只增 G01 content/delivery/renderer；`FrameApplied` 后清 draft；DOM 覆盖 Sending/Sent/Failed 与 bad-frame unchanged。
4. **Cross-system validation and deletion**: 两用户、SQLite/PG read-only、retry/timeout/stale binding 反证；T039 审查准入后 T040 执行 legacy producer physical deletion，再由 T041 回归取证。Mobile 只复用 001 已验收的 fixture/cache 基线，不在 002 新建基础工作。

## Complexity Tracking

无 Constitution 违例。九个 Surface 是有限、独立 revision 的 frame slots，不是九套 Store；G01 只 compose 事实确实影响的 slots。
