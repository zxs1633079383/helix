# Research: G-07 加急 + 确认

**Source Lock**: Helix `ccbc354ab2444736400ad45195556fecffce7b94`；cses-client `ab7833996be5156f4d0e0655c18e7f9c65488961`；Mobile `3bd7650a2545e4a8a2d56b552d2ad20ee1bbce8e`

## Decision 1 - 唯一业务真源

**Decision**: `G-07` 的 command（im_urgent_post、im_urgent_confirm）、effect（http_write、projection）、持久事实（message、message_receipt）与 Surface（message.timeline、message.participant-receipts）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G07_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/urgentPost` body `{channelId,postId,targetIds,message?}`；`POST /api/cses/posts/urgentConfirm` body `{postId,channelId}``；WS 为 ``post_update`, `update_channel``；远端表为 `post、channel、channel_member、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/outbound/engagement/urgent.rs`；现有聚焦测试：`crates/helix-im/tests/g07_urgent_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc08UrgentTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## T002 Current / Target / Gap Revalidation

### CURRENT

- Helix `urgent.rs` 已有 `im_urgent_post`、`im_urgent_confirm` 与
  `im_urgent_cancel` canonical wire；post 只发 `{postId,targetIds,message?}`，
  confirm/cancel 只发 `{postId}`，频道与 actor authority 留给服务端。
- `g07_urgent_interaction_test.rs` 仅有 `2` 个聚焦合同测试：三个 live
  ActionBinding 与 outbound forbidden actor 字段；尚未覆盖生产
  `ImModule::handle` 的 Persist/HTTP/WS/Frame 全流程。
- PC 已注册 Tauri urgent commands 并能渲染 timeline 的 urgent 字段，但没有
  G07 canonical mirror、独立 Mock/Real/report lane 或 participant-receipts
  字段级合同。
- Mobile SDK/QuickJS 已有 urgent post/confirm admission，SurfaceRuntime 已识别
  `message.participant-receipts`；现有 UC08 仍等待已被 V2 consumer 隐藏的
  `im:post:updated` lifecycle event，且未通过 typed view 验收确认终态。

### TARGET

- `G07ScenarioRunner` 只通过 production `ImModule::handle`、matched
  PortReply/Inbound 与 `Effect::Emit` 生成 timeline +
  participant-receipts canonical Frame。
- PC/Mobile byte-exact 镜像同一 Fixture，分别由真实生产
  SurfaceRuntime 原子 apply；平台不得重建 urgent aggregate、receipt 或权限。
- PC 关闭 Mock、single-real、dual-real、report；Mobile 关闭 GTest、QuickJS
  typed view 与 focused real-chain。

### GAP

- 缺 Helix full-flow ScenarioRunner、race/recovery 断言、canonical Fixture 与
  provenance。
- 缺 PC G07 fixture loader/bridge/page/spec/report/boundary 独立证据。
- 缺 Mobile G07 Frame 合同和 typed UC08 real-chain；需去除 V1 lifecycle
  wait，但不得新增 JS raw event reducer。
