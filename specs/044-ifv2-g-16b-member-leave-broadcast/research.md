# Research: G-16b member leave 广播

**Source Lock**: Helix `71d1350b330449ddb1d61b48a854b31c7640f18f`；cses-client `aac3c1583519b9308acb7c8dd0e415fa1dfc3813`；Mobile `46455d4d4e88870b75e72fdf1caa9f518eb87835`

## Decision 1 - 唯一业务真源

**Decision**: `G-16b` 的 command（im_channel_leave）、effect（http_write、transport_broadcast、projection）、持久事实（channel_member）与 Surface（message.member-roster）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G16B_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/channel/member/leave` body `{channelId,userId?,newCreator?}``；WS 为 ``channel_member_update`, `quit_company`（公司域也可能推）`；远端表为 `channel_member、channel、channel_event、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Current / Target / Gap

- **CURRENT**: `im_channel_leave` outbound wire 真实 owner 是
  `crates/helix-im/src/outbound/channel_admin/existing.rs`，G16 已在
  `CorrelationContext::OutboundLeave -> ChannelLifecyclePersist` 后产出
  `message.channel-summary lifecycle=left`。它不是 G16b roster 广播的完成信号。
- **CURRENT**: WS `channel_member_update` 由
  `crates/helix-im/src/ws/handlers/channel_member_update.rs` 收集 channel/member 写集，
  其中 `memberChange.leave` 编译为 scoped `BatchDelete(channel_member)`；当前写集使用
  `PersistFire`，并在持久回执前发布 raw `im:channel:member-updated` 与
  `im:channel:members`，尚未生产 `message.member-roster` InteractionFrame V2。
- **CURRENT**: `MemberRosterProjector` 已支持 snapshot 与带 `baseRevision` 的完整
  `replaceView` patch；`SurfaceRuntime` 已能从 `channel_member` Scan 生成 typed roster，
  但 WS member update 尚未在 durable barrier 后触发该 readback。
- **TARGET**: WS authority 的 channel/member/leave 写集先经有回执的原子持久化；成功后只为
  已 attach roster slot 发起权威 readback，并从生产 projector 生成完整 Frame。未 attach
  surface 不伪造 Frame，后续 attach 可从 durable fact 恢复 snapshot。
- **GAP**: 缺 G16b 专属 persist correlation、attached roster refresh、patch 选择、
  happy/failure/duplicate/recovery ScenarioRunner 与 canonical Fixture；PC/Mobile 也尚无
  G16b mirror、typed consumer Gate 和真实链报告。

## Resolved Scope

- 当前 outbound wire owner：
  `crates/helix-im/src/outbound/channel_admin/existing.rs`；当前 WS authority owner：
  `crates/helix-im/src/ws/handlers/channel_member_update.rs`；现有聚焦测试：
  `crates/helix-im/tests/s1_ws_actions_test.rs` 与
  `crates/helix-im/tests/g16_governance_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc16ChannelLifecycleTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
