# Research: G-S2 sync notify

**Source Lock**: Helix `918999d649007d92b085d8f5aba73329c9241ee4`；cses-client `13e42ef2793ea5c9f39b29f5d1ce86b1aca074db`；Mobile `674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`

## Decision 1 - 唯一业务真源

**Decision**: `G-S2` 的 command（im_sync_channels）、effect（sync_request、projection）、持久事实（channel_event_cursor、message）与 Surface（message.left-dialog-list）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_GS2_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}`，单次最多 500 个 cursor`；WS 为 `HTTP `/channel/sync/notify` 本身不产生业务 WS；在线并发的 `post` / `posts_update` / `increment_channel_end` 必须作为独立 transport evidence，不能替代 HTTP entries、本地持久化或最终投影`；远端表为 `channel_event、post、channel、channel_member`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/sync/explicit.rs`；现有聚焦测试：`crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/GsSyncPrerequisiteTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
