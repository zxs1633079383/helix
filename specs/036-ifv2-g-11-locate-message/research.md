# Research: G-11 按 postId 定位

**Source Lock**: Helix `8c732a84cf0c0d29e65646e3aa5bfd7e87675856`；cses-client `f1c4ddc0fbbd4bb4547108627af83a028284a343`；Mobile `26c7616d805f0ba21406f0f9fa3579e212bdd090`

## CURRENT / TARGET / GAP

**CURRENT**:

- Helix direct command 与 opaque action 都把 `im_get_posts {post_ids,req_id,locate:true}` 登记为
  `OutboundLocatePost`，HTTP success 只调用 `TimelineProjector::patch_located_target`。
- 当前 patch 仅在目标已经存在于 attached timeline 时生效，但没有校验 HTTP body 中的目标身份和
  channel；同时仍发布 `im:read:result` 与 `im:post:located` raw 事件。
- 当前 typed Frame 含 `anchor.mode=locate` 与 `targetMessageId`，但 `directives=[]`；PC 真实场景仍等待
  `im:post:located`，Mobile UC11 仍等待 raw event、读取 `im.store`，没有通过
  `im.surface.view` 验证目标。
- fresh-group 前置已由 Helix `G01ScenarioRunner::create_group_prerequisite`、PC
  `prepareMockCreatedGroup/createGroupThroughUi` 与 Mobile `RC.createChannel` 共享实现；G11 只需复用，
  不新增第二套建群领域逻辑。

**TARGET**:

- HTTP reply 必须包含当前 attached timeline 中目标 post，且其 `channelId` 与目标所在 scope 一致；
  缺目标、跨 channel、畸形 envelope 或 transport failure均 fail closed。
- 成功只发布一个生产 `InteractionFrameV2`：同一 Frame 原子携带完整 timeline patch 与 payload-free
  `revealMessage` directive；raw `im:read:result`/`im:post:located` 不再是 G11 consumer 合同。
- 目标消息在本 Gate 前已经由 G01/G10 权威持久化并进入 attached bounded window，因此 G11 是纯读
  authority，不伪造额外 message 写屏障；不存在目标窗口时不得向端侧插入孤立 row。
- PC 与 Mobile 只 apply 同源 Frame；PC 通过 DOM `data-highlighted=true`，Mobile 通过
  `im.surface.view` 的 `targetMessageId`、anchor 和同一 item identity 验收。

**GAP**:

- Helix 需补 HTTP body identity/channel 校验、同 Frame directive、raw 成功旁路移除及完整
  ScenarioRunner/Fixture。
- PC 需把旧 raw projection 等待改为 FrameApplied/typed DOM correlation，并补 Mock/Real/report。
- Mobile 需同步 Fixture，增加 production-linked Frame GTest，并把 UC11 real chain 从 raw event/
  `im.store` 改为 QuickJS typed view。

## Decision 1 - 唯一业务真源

**Decision**: `G-11` 的 command（im_get_posts）、effect（http_read、storage_upsert、projection）、持久事实（message）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G11_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 `POST /api/cses/posts/get` body `{postIds:[...]}`；WS 为 `无`；远端表为
`post`。目标已在前置 Gate 的 durable bounded timeline 中，G11 的 terminal authority 是 HTTP body
中同一 post/channel 身份与当前 attached view 的交集，不新增伪 message 写；transport success 本身
不等于 locate 成功。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/query/dispatch.rs`；现有聚焦测试：`crates/helix-im/tests/g11_locate_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc11LocateTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
