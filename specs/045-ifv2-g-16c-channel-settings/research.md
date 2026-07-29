# Research: G-16c 群属性修改

**Source Lock**: Helix acceptance `a6f5bacadeb2c13944f292d46e58dd441d27ffbe`；
cses-client `48c48b04e6eb980334bade119fb34b859b1e435a`；Mobile
`1b0b11d33193a80c362491127a9d8d1e5227008c`；Go
`870200d2931494da7e3fc3ac3a47e80e6646ebe4`。

## Decision 1 - 唯一业务真源

**Decision**: `G-16c` 的 command（im_channel_change_display_name、im_channel_change_notice、im_channel_change_info）、effect（http_write、projection）、持久事实（channel）与 Surface（message.channel-summary）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G16C_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/channel/change/info` body `{channelId,name?,displayName?,header?,purpose?,orient?,module?,pictureType?,picture?,source?,expectedVersion?}`；其他专用命令见 UC-5.5`；WS 为 ``update_channel`, `update_channel_notice``；远端表为 `channel、channel_event、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/outbound/channel_admin/change.rs`；现有聚焦测试：`crates/helix-im/tests/g16_governance_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc16ChannelLifecycleTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## Implementation Revalidation

- Helix 生产链由 `change.rs` 的 command/effect/persist barrier 与
  `surface_composer.rs` 的 `message.channel-summary` snapshot/patch 组成；
  canonical Fixture producer 固定在 `32740014cf30ed92998ea89a504597fbaef6e7b5`。
- Go 提交后广播由 `internal/app/post_send_publish.go` 回读 committed channel 与
  current members；幂等重放只使用持久 post ID 回读的 Type/Props 分类，拒绝 replay
  payload 伪造 `join/channelUpdate`。
- PC 与 Mobile 既有 production runtime 已具备 typed Frame apply；实现只增加
  command routing、机械消费、同源 Fixture 与验收，不新增平台业务 reducer。
