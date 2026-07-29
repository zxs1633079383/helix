# Research: G-04a 离线撤回重放

**Source Lock**: Helix `394839c76be083947bbbb2993286e4582055a50b`；cses-client `c91cafc8359495daff2969ca4ed0db729027829f`；Mobile `094a15149f448b7ce952b71688e5bae9fe278769`

## Decision 1 - 唯一业务真源

**Decision**: `G-04a` 的 command（im_sync_channels）、effect（sync_request、storage_upsert、projection）、持久事实（message、channel_event_cursor）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G04A_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 `无 UI 主动 HTTP；由 `channels/load/increment` / `sync/notify` 拉到事件`；WS 为 ``increment_channel`, `posts_update`, sync events`；远端表为 `channel_event、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Current Code Map

- Public command normalization/dispatch 已存在：
  `client_api/command.rs -> module/trait_impl.rs -> sync/explicit.rs`。
  `sync/explicit.rs` 只解析 `channel_id`，未知 channel 注册 cursor=0，已知 channel
  使用 Helix 本地 cursor，并通过 `sync_scheduler` 发
  `/channel/sync/notify`；payload 中的 `eventSeq` 不是 authority。
- HTTP reply owner 是 `sync/flow.rs::handle_sync_reply`，不是
  `sync/increment.rs`。后者只负责 heartbeat cursor/allHash，不能承担 G04a
  command/reducer。viewer-filtered events 先排序、校验 contiguous facts，再把
  message、recovery ledger、coverage、cursor 与 dialog unread 组成一次
  `PersistAtomic`。
- `CorrelationContext::ChannelPersist.pending_domain_events` 在 PersistOk 前只缓存
  producer event；因此 offline `posts_update/revoke` 不能提前产生 recalled Frame。
  `g04_revoke_interaction_test.rs` 已证明直接 inbound fact 无 HTTP、只有一次
  PersistAtomic 且 PersistOk 前无 Emit，但尚未覆盖完整 explicit sync PortReply。
- timeline projector 已将 durable `revoke=true` 投影为
  `content.kind=recalled`、`delivery.kind=recalled` 且 actions/quickActions 清空。
- PC 当前没有任何 `g04a` Fixture、loader、WDIO lane 或 reporter；G04 的 online
  revoke 资产只能作为 harness 参考，不能复制它的 HTTP action 顺序。
- Mobile `Uc05RevokeTest` 已有 `im_sync_channels` command admission 与旧 real-chain
  观察，但当前 G04 contract 只覆盖 online revoke；尚无 G04a canonical mirror、
  production-linked `G04AFrameContractTest` 或 typed recovery view。

## Target And Gap

- 新增 `G04AScenarioRunner`，从 fresh group + revocable message 开始，喂入生产
  `im_sync_channels`，捕获 sync/notify corr，再用真实 Go wire envelope 的
  `PortReply` 回放 revoke event，经过 PersistAtomic/PersistOk/readback 后捕获完整
  InteractionFrame V2。
- required cases：happy offline replay、NoChange/business no-op、HTTP failure、
  persist failure/replay、duplicate/stale reply、late duplicate fact 与 cold recovery。
  任一 transport reply 都不能在 durable barrier 前发布 recalled。
- canonical Fixture 与 G04 分目录；PC/Mobile 只按 provenance 机械 rebase/apply，
  不从 `im:messages:query_result`、raw `posts_update` 或本地 row 重建终态。
- 当前 Helix production chain 具备主要 reducer；若完整 Scenario 暴露
  PortReply/ChannelPersist 后未触发 timeline readback/Frame，缺口必须回
  `sync/flow.rs` 或统一 surface composer 修复，禁止在测试 runner 补 Emit。

## Resolved Scope

- 当前 Helix owners：`crates/helix-im/src/sync/explicit.rs`、
  `crates/helix-im/src/sync/flow.rs` 与 PersistOk dispatch；`sync/increment.rs`
  仅是 heartbeat 辅助真源。
- 现有聚焦测试：`crates/helix-im/tests/explicit_sync_command_test.rs` 与
  `crates/helix-im/tests/g04_revoke_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc05RevokeTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
