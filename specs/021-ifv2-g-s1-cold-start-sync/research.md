# Research: G-S1 hello 全量增量

**Source Lock**: Helix worktree
`3ddce5e9b69eb52355d631bae41ea02e09074ea8`；cses-client worktree
`d0d06d87f406646d865cbce07434d5080558a2ae`；Mobile worktree
`a3580948fa1f115d3e19ac38d27ac624d601d481`；Go worktree
`f2e919c6ea95a898d25827a2a82ea991a3ad0bf3`。PC 与 Mobile 均正式 pin
`98c522a3621cc599c134cb78b9e4656668eb67bb`；该 pin 是 Helix 当前 HEAD
的祖先，且二者在 `helix-core`、`helix-im`、`helix-driver-host`、
`helix-driver-native` 上无代码差异。

## Decision 1 - 唯一业务真源

**Decision**: `G-S1` 的 command（im_query_dialog_list）、effect（local_scan、projection）、持久事实（channel、channel_member、channel_event_cursor）与 Surface（message.left-dialog-list）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_GS1_WDIO_FIXTURES=1` 才更新。

**Rationale**: Fixture 是 Helix producer 与两个平台 consumer 的跨仓合同，不是 command 同步返回值。

**Alternatives considered**: 从 DOM 反推 JSON、复制其他 Gate Fixture、前端补动态业务字段；全部拒绝。

## Decision 3 - PC 验收分层

**Decision**: PC 分开执行 Fixture parity、Mock replay、single-real、按用户可见影响决定的 dual-real、DOM/FrameApplied 断言和结构化报告。

**Rationale**: Mock 不经过 Rust/Tauri/SQLite/HTTP/WS，不能替代真实链。

**Alternatives considered**: 只跑 Mock、只证明 invoke accepted、固定 sleep 等第三帧；全部拒绝。

## Decision 4 - Mobile 生产接线

**Decision**: Mobile GTest 链接生产 `MobileImSdk/MobileFfiBridge/SurfaceRuntime/QuickJS binding`，完整 Frame 经 DomainEvent batch 原子 apply，JS 只通过 `im.surface.view` 获取 typed view。

**Rationale**: Mobile 不允许 `im.store`、raw reducer 或测试注入 API进入默认生产构建。

**Alternatives considered**: JS 直接消费 raw event、GTest 注入私有 reducer、每 mutation 一次 FFI callback；全部拒绝。

## Decision 5 - Gate-specific authority

**Decision**: HTTP 为 ``POST /api/cses/channels/load/increment` body `{timestamp,cursors:[{channelId,fromSeq}]}`；单频道 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}``；WS 为 ``hello` 建立连接；零个或多个 `increment_channel.data` 承载频道快照，服务端水位读取成功时包含 `lastEventSeq`，根群 cursor 对账时包含 `needSync`；同一连接最后收到 `increment_channel_end` 只证明该批 WS 入队结束，不证明本地持久化或最终投影完成`；远端表为 `channel、channel_member、channel_event、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/sync/increment.rs`；现有聚焦测试：`crates/helix-im/tests/gs_persist_barrier_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/GsSyncPrerequisiteTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## Implementation Revalidation

- `sync/increment.rs` 负责 hello 后单次 increment HTTP；逐条 WS
  `increment_channel` 只缓冲，`increment_channel_end.rs` 将
  `channel -> channel_member -> channel_event_cursor` 组成带 corr 的持久事务。
- `gs_persist_barrier_test.rs` 已证明 HTTP success、逐条 increment 与 global end
  都不能越过 PersistOk；现有缺口是将最终 durable dialog projection 导出为 GS1
  canonical Frame 与三端同源验收，不是再造同步状态机。
- Mobile `GsSyncPrerequisiteTest.cpp` 已有 fresh group 和 `im.sync.dialogList()`
  生产入口；本 Gate 只补 complete DomainEvent batch 与 typed
  `message.left-dialog-list` 合同。
