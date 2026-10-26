# Implementation Plan: SPEC-IFV2-GS01-GS04-同步恢复

**Branch**: `main` | **Date**: 2026-07-21 | **Spec**: [spec.md](./spec.md)

**Input**: [GS01-GS04 同步到 Frame 完整时序与数据](../../crates/helix-im/docs/interaction-v2/01-GS01-GS04-同步到Frame完整时序与数据.md) 与 [V2 总览及硬切准入](../../crates/helix-im/docs/interaction-v2/00-首批纵切总览与硬切准入.md)。

## Summary

将重连与重新登录后的同步恢复收敛为一条 V2 纵切：`HostObservation/Tick -> sequence compare -> SyncBatchFacts -> PersistAtomic -> PersistOk -> SurfaceComposer -> InteractionFrame v2 -> Frame Cache atomic apply -> directive/ack`。`InteractionFrame v2` 是唯一业务 UI 下行真源；raw `im:*` 投影 emit 不得成为 V2 consumer 的回退输入。

本 Feature 实现 GS01-GS04，覆盖九个 Surface 中受同步影响的快照和补丁：现有八个 Surface，加 `message.composer@Channel(channelId)`。目标不是把原始 event 搬到三端，而是让 `helix-im` 依据已提交事实选择受影响且已 attach 的 Surface，并让 PC/mobile 机械校验、暂存、原子替换和 ack。

## Technical Context

**Language/Version**: Rust（workspace 当前 toolchain），TypeScript/Angular（`cses-client` message-v4），C++/QuickJS mobile consumer（合同和 fixture 消费）

**Primary Dependencies**: `helix-core` 的 Tick/Effect/Storage ports；`helix-im` 的 sync、gate、projector、interaction frame；`helix-driver-host` 的 SQLite `atomic_write`；Angular signals/SurfaceRuntime；现有 readonly SQLite/PG MCP

**Storage**: 本地 SQLite 的 `channel`、`channel_member`、`message`、`channel_event_cursor`，新增本地 `channel_event_ledger` 与 `channel_sync_coverage`；远端 PostgreSQL 的 `channel`、`channel_member`、`channel_event`、`channel_event_head`、`post`

**Testing**: Rust `cargo test -p helix-im`、`cargo test -p helix-driver-host`、确定性 Tick/replay fixtures；cses-client Jest/TestBed DOM；PC/mobile 共享 Frame fixture；SQLite/PG readonly MCP 对账

**Target Platform**: native/Tauri PC、Flutter FFI mobile、Web Rust shell 的同一 `helix-im` 业务语义；首批 UI 实施和 DOM 证据在 PC，mobile 以同 wire fixture/consumer contract 验证

**Project Type**: 跨仓 runtime-agnostic Rust 业务模块和两端 renderer consumer

**Performance Goals**: 每个同步批次一次 `PersistAtomic`；本地事件账本按 `(tenant_id, channel_id, event_seq)` O(1) 写入和查询；同一 Frame 按附着 slot 数 O(k) 暂存/替换；不对未 attach Surface 组装 VM

**Constraints**: `helix-core` 不认识 IM/UI schema、无 runtime/DB 依赖；业务 schema 只在 `helix-im`；SQLite/PG 证据只读；持久失败不推进 cursor/Frame；UI 同步中不冻结有效频道操作；无 raw-event renderer fallback；连续工作超过 30 分钟必须报告进度

**Scale/Scope**: GS01-GS04 的恢复、事件账本、同步相关 composer、九 Surface 合同与 PC/mobile 消费；G01-G16c 不在本次实现范围，只复用既有 V2 合同

## Domain Ownership And Boundaries

| 所属 | 负责 | 不负责 |
|---|---|---|
| `helix-core` | 运输不透明 Tick、Effect、`StorageOp`、相关键和原子写端口 | channel、message、cursor、Frame/VM、SQL schema |
| `helix-im` | sequence compare、sync 状态机、事实归一、账本写集、SurfaceComposer、ActionBinding、Frame v2 | SQLite 连接、HTTP/WS 实际 I/O、具体 UI |
| `helix-driver-host` / native | 一个 SQLite 事务兑现 `PersistAtomic`、执行 IM 提供的 DDL、结构化 hop | 解读 IM 事实或决定 UI |
| `cses-client` | V2 decode、Frame Cache 校验/暂存/一次替换、directive/ack、DOM 渲染 | 从 raw event 组装 Row、重算排序/未读/权限 |
| mobile | 复用 V2 wire、同样 cache 语义与 renderer 映射 | 单独的 sync/reducer 业务分支 |
| Go/PG | 现有权威事件与事实的只读证据面 | 本 Feature 的客户端/UI 逻辑；仅 wire 缺口时条件介入 |

## Constitution Check

`/.specify/memory/constitution.md` 仍为未填充模板，因此此计划以仓根 `AGENTS.md`、`docs/GOAL.md` 和 `HX-C001/C005/C007/C018` 为有效约束。

### Pre-design Gate

| Gate | Result | Plan response |
|---|---|---|
| Runtime-agnostic / dependency direction | Pass | 所有 IM 状态、账本规格、composer 和 Frame 类型留在 `helix-im`；core 只传递通用 `StorageOp`。 |
| Deterministic / sans-IO | Pass | 时间、correlation、身份均由 Tick/host 注入；remote compare 和 DB I/O 通过 Effect/PortReply。 |
| Atomic persistence before UI | Pass | 同批事实、ledger、coverage、cursor 一次 `PersistAtomic`；仅 `PersistOk` 后 compose。 |
| Hot-path and boundedness | Pass | 表/列常量、批量 upsert、复合主键、O(k) Frame staging；禁止 read-modify-write 和全表 UI 重建。 |
| Headless interaction boundary | Pass | V2 只发语义 VM、ActionBinding、Directive；平台只做 renderer mapping、cache、焦点/提示实现。 |
| Cross-platform batch boundary | Pass | Frame/ack 批量跨 native/FFI/Web，禁止逐项 renderer callback。 |
| Evidence integrity | Pass | SQLite/PG MCP 只读；不能以 hand-edit cursor/ledger 伪造通过。 |

### Post-design Re-check

通过。设计没有新增 core 业务依赖、平台 UI 细节或 raw emit fallback。`channel_event_ledger`/`channel_sync_coverage` 是 IM-owned schema：现有 `crates/helix-im/src/schema.rs` 以 `IM_SCHEMA` 将业务 DDL 作为数据交给 host 执行；driver 只执行 DDL 和通用 `StorageOp`，不得解释 event 语义。

## Architecture And Flow

1. `Tick::Connected`、hello 或重新登录身份切换创建新的 `SyncRecoverySession`。旧身份的 action/Frame cache scope 通过 session epoch 失效。
2. `Scan(channel_event_cursor)` 获得本地**已提交**水位；`LoadIncrement`/`SyncNotify` 取远端权威水位和事实。相等只结束比较，不重放业务记录。
3. 解析器把 channel、member、message、viewer state 与 event 归一为 `SyncBatchFacts`。它先校验 event 连续性、身份和 scope；未知/过长/本地高于远端走权威重载或 `BLOCKED`，不猜测补洞。
4. `helix-im` 生成同一个 `PersistAtomic` 写集：事实投影、`channel_event_ledger`、`channel_sync_coverage`、最后的 `channel_event_cursor`。任一写失败，事务回滚，状态保留 `syncing/failed`，不产最终业务 Frame。
5. `PersistOk` 后 `SurfaceComposer` 根据 attach registry 读取 committed facts。它只组合受影响 Surface；用户 scope 的 dialog/unread 与 runtime 同帧，已打开陌生频道的 `channel-summary + composer` 同一 channel bundle。timeline/thread 仅在 attach/window 命中时出现。
6. 生成 `InteractionFrame v2`：`schemaVersion=2`、事务/causation、runtime、唯一 slot mutations 和 directives。PC/mobile 先完整 decode 和 revision/epoch staging，再以单次 cache version swap 发布所有 slot，随后执行 directive 并发送 ack；失败则 scope-resync，不消费 raw event。

## Project Structure

### Documentation

```text
specs/001-gs-sync-recovery/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── interaction-frame-v2-sync-recovery.md
│   └── sync-recovery-evidence.md
└── tasks.md
```

### Implementation Targets

```text
crates/helix-im/
├── src/
│   ├── sync/{flow.rs,session.rs,increment.rs,scheduler.rs}
│   ├── channel/gate/{flow.rs,storage.rs,commit.rs,projection.rs}
│   ├── interaction_frame.rs
│   ├── module/{action.rs,*.rs}
│   └── projector/
└── tests/{gs01_04_v2_sync_recovery_test.rs,interaction_frame_*}

crates/helix-driver-host/
├── src/storage/{connection.rs,write.rs}
└── tests/{storage_conformance.rs,im_event_persist.rs}

/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/
├── surface-runtime/{surface-contract.types.ts,surface-contract.codec.ts,surface-runtime.service.ts}
└── surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts

/System/Volumes/Data/workspace/c/mobile-qucik-c++/
└── <existing InteractionFrame consumer and fixture test owner; resolve in T022>
```

**Structure Decision**: 保持现有 core/module/driver/shell 分层。V2 是 `helix-im` 的业务协议升级，不新建第二个同步服务或端侧 reducer；storage adapter 增加的是通用迁移/事务保证，PC/mobile 只扩展同一 Frame cache consumer。

## Delivery Slices

| Slice | Includes | Exit condition |
|---|---|---|
| Foundation | V2 typed contract、nine surfaces、ledger/coverage schema、persist barrier、fixture baseline | 能用 deterministic test 证明失败不前进 cursor/Frame。 |
| US1 / GS01-GS02 | reconnect compare、increment/sync notify、runtime/global syncing、dialog/unread Frame | 本地低水位恢复为远端水位，PC DOM 同时见同步状态和恢复视图。 |
| US2 / GS04 identity | session epoch、identity reset、no duplicate state | A->B 重新登录无旧 action/Frame/ledger scope 泄漏。 |
| US3 / GS03 | unknown-channel hydration、summary+composer bundle | hydration 前无发送 action，提交后同帧出现 summary+composer。 |
| Acceptance | readonly evidence pack、PC/mobile fixture parity、raw producer deletion readiness | 证据四面齐全；物理删除 raw producer 是后续独立 change。 |

## Complexity Tracking

| Additional element | Why needed | Simpler alternative rejected because |
|---|---|---|
| `channel_event_ledger` + `channel_sync_coverage` | 证明 cursor 覆盖的远端事件完整、无重复地应用 | 仅最大 cursor 无法解释已应用范围，违反 FR-005。 |
| Ninth `message.composer` surface | 让频道发送能力有稳定 scope/epoch/actionRef，避免编辑器从 timeline 或 summary 猜能力 | 将 composer 字段塞进 message/channel VM 会让新增消息类型和编辑器能力交叉膨胀。 |
| Runtime mutation in V2 Frame | 全局同步状态必须和已提交 Surface 同一次 cache swap | 单独 raw `im:sync:*` 会重建第二业务真源并可能先于持久提交显示成功。 |
| Shared frame fixtures | PC/mobile 机械消费同一业务结果 | 各端用独立 mock event 容易重新实现 sync/reducer 语义。 |
