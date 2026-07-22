# Research: SPEC-IFV2-G01-在线发送

## R1. 已发送的权威判定

**Decision**: `Sent` 只由匹配请求的 WS echo 或权威同步事实，在 canonical 持久化 `PersistOk` 后产生。

**Rationale**: HTTP 2xx 仅代表 transport accepted；若把它当成已发送，网络延迟、服务端异步失败或重复事件会让 UI 错报成功。Persist 屏障还保证重启后能恢复同一 delivery state。

**Alternatives considered**: HTTP 2xx 直接 `Sent`（违反 FR-005）；客户端计时后自行 `Sent`（没有权威事实）；echo 先 frame 后 persist（重启/失败不可恢复）。

## R2. 交互形状与 VM 演进

**Decision**: 使用 versioned、discriminated `InteractionFrameV2`、`MessageRowVmV2`、`VersionedMessageContent`、`ActionBinding`、`UiDirective`；禁止 V1 `SurfaceDataVm`/`unknown` 通过 V2 业务边界。

**Rationale**: typed contract 能对 unknown surface/content、wrong scope、revision gap fail-closed。消息种类在 content codec registry 扩展，不让 composer 或 timeline 根 VM 无限堆字段。

**Alternatives considered**: generic JSON/`SurfaceDataVm`（让客户端重担业务猜测）；每类消息加 composer 根字段（随消息类型线性耦合增长）。

## R3. Frame Cache 与 directive 顺序

**Decision**: V2 frame 所有 mutation 与 runtime 在临时副本验证后原子提交，随后 `FrameApplied`，再按序执行 `ClearComposerDraft` 等 directive 并 `DirectiveApplied`。

**Rationale**: 清输入早于显示 Sending 会在 persist/apply failure 时丢字；directive failure 不应回滚已持久化业务事实。`ActionReceipt` 只改变 local overlay。

**Alternatives considered**: action accepted 即清业务 cache/draft（PersistErr 无法恢复）；mutation 逐条提交（产生半帧矛盾）。

## R4. 连续发送与 pending 状态

**Decision**: 每次 accepted invocation 有唯一 `requestId` / temporary identity，pending ledger 按 request 保持到终态；不新增用户级发送上限，也不把 actionRef 当全局单飞锁。

**Rationale**: 用户已允许连续发送。防重复对象是同一 request/echo/canonical identity，不是阻止不同文本的合法并发。actionRef 的 epoch/scope/revision 仍须 fail-closed。

**Alternatives considered**: 一频道一条发送强制队列（改变产品语义）；仅文本 hash 去重（相同文本可能是两次合法发送）。

## R5. raw emit 硬切

**Decision**: G01 migration 完成后物理移除 `im:post:sending` 等 legacy producer、相应 G01 fixture 预期和客户端 raw reducer 路由；不 dual-write，不留 fallback。

**Rationale**: 旧 emit 即使 message-v4 不消费，仍可能污染其他 UI 和诊断面。两条真源会让行重复、顺序漂移、actionRef 生命周期无法证明。

**Alternatives considered**: 一段时间 dual-write（用户已要求 v2 唯一真源）；只停消费但留 producer（继续污染，无法证明下行完整）。

## R6. 数据库核验和修复权限

**Decision**: `mcp__helix_local_sqlite_ro`、`mcp__cses_pre_postgres_ro` 用 `corrKey`/channel/user 的 bounded `SELECT` 取证；写入只能走正常 Effect/产品链。缺陷修 source/test 后重跑。

**Rationale**: direct DB repair 能伪造通过，破坏因果证据。当前本地/远端表名不同必须如实记录，不能补写不存在的表。

**Alternatives considered**: 测试前 SQL cleanup/repair（违反 scope）；只看 DOM（不能证明 local/remote facts）。

## R7. 跨 Surface 判定

**Decision**: composer owns capability；timeline owns delivery row；dialog owns preview/sort；thread 只在 reply 且 attached 时更新；receiver 仅在 canonical fact 落库后更新自己的 slots。

**Rationale**: “消息影响哪一个 Surface”由 committed facts 与 attached slots 的交集决定，不按事件名字猜。没有 unread fact 就不生成 unread mutation。

**Alternatives considered**: 每次发送刷新全部九个 Surface（无意义 mutation/失效 binding）；UI 从 timeline 事件推 dialog/unread（重建客户端业务 Store）。

## R8. GS01-GS04 基线准入

**Decision**: 002 在 001 的 T050 为**当前 source pin**记录 Rust、PC DOM、mobile fixture、SQLite/PG readonly 四面 `PASS` 前，只维护文档和只读审计；不创建实现 worktree、不运行实现命令、不改 fixture 或源码。

**Rationale**: nine Surface、公共 camelCase wire、Frame Cache 原子 replace、通用 fixture grammar、hydration 后的 summary+composer/action binding 是 001 的共享交付。若 G01 在 001 未验收时平行重写，会产生两个 cache/codec/binding 真源，并把 actionRef 失效、恢复及跨端表现的根因混在一起。

**Alternatives considered**: G01 先自行补 shared V2 基础（重复所有权且易冲突）；只引用旧 source pin 的绿灯（不能证明当前代码）；001 “正在跑”即启动实现（缺少可复核完成证据）。
