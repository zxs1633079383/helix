# Research: G-15 创建话题

**Source Lock**: Helix `836898c8ed4d09eea39f365defef53c062df5723`；cses-client `01ee73c430c7ab39ab14411b4d0eae7f8b361950`；Mobile `afa41cd8d2dd6567a42d258c3ba504a0210f1125`；Strict Matrix `84b9cc6ce2bd3c7ffe4595371354eb609517b89902228bd27cfb93e85b80c770`

## Decision 1 - 唯一业务真源

**Decision**: `G-15` 的 command（im_make_topic）、effect（http_write、storage_upsert、projection）、持久事实（channel、message）与 Surface（message.channel-summary、message.thread-detail）只由 Helix 生产路径决定。`reply_tree` 是现有 raw `im:channel:replies` consumer 的兼容投影，不是当前 Helix make-topic 持久写；本 Gate 必须用 typed Frame 取代该 raw 渲染合同，而不能在平台补 row。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G15_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位`；WS 为 ``channel_created`, `post_update``；远端表为 `channel、channel_member、channel_topic、post、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Current / Target / Gap

### Current

- command admission 位于 `crates/helix-im/src/module/trait_impl.rs`，wire command 位于 `crates/helix-im/src/outbound/channel_admin/existing.rs`；`crates/helix-im/src/channel/gate/flow.rs` 是通用 channel ingest gate，不是 `im_make_topic` owner。
- HTTP authority 回灌与持久屏障位于 `crates/helix-im/src/port_reply/dispatch_projection.rs`：成功响应先写 topic `channel` row 与 root `message.topic`，matching `PersistOk` 后才进入 terminal projection。
- 当前 terminal projection 仍是 raw `im:channel:replies`；`uc15_16_projection_test.rs` 只证明 raw replies 在 persist barrier 后发布。PC 的 `surface-runtime-hard-cut.test.ts` 已明确拒绝该 raw event，Mobile `Uc15ChannelCreateTest.cpp` 仍保留 raw reply-tree characterization，因此三端当前不一致。
- `channel_created` 是独立 WS authority：它可以通过现有 channel gate 产生 topic channel 的 `message.channel-summary`。HTTP terminal 与 WS channel summary 是两条可竞态 correlation，不能写死为固定先后。
- Angular 真实入口已经存在：`MessageUiCreateTopicComponent -> ShellService.createTopic -> im_make_topic`；Mobile production admission 已存在：`MobileImSdk::makeTopic`、QuickJS `im.channels.makeTopic`。

### Target

- HTTP success 只在 channel/message durable 后发布 `message.thread-detail` typed Frame；Frame 的 topic context 关联 `topicChannelId/rootMessageId`，不再发布 raw replies。
- 独立 `channel_created` WS authority 继续通过生产 channel gate 发布 `message.left-dialog-list` 与完整 `message.channel-summary`/`message.member-roster` Frame；测试分别验证两条 correlation 的竞态、幂等与 recovery，不把它们伪造成单一固定帧序列。
- Helix canonical Fixture 是上述生产 `ImModule::handle -> Effect::Emit` 的唯一真源；PC/Mobile byte-exact mirror 并机械 apply。
- Mobile 删除 G-15 raw `reply_tree_row` characterization，改为链接 production SDK/Bridge/SurfaceRuntime/QuickJS binding 的 typed `im.surface.view` 合同。

### Gap

- Helix 尚无 G-15 ScenarioRunner、完整 flow test、Fixture contract 与 canonical JSON。
- `handle_make_topic_persist_reply` 尚未生成 InteractionFrame V2，且持久失败/late reply/duplicate authority/recovery 的字段合同不完整。
- PC 尚无 G-15 fixture mirror、typed loader、Mock/Real/PageObject/report/boundary lane。
- Mobile 尚无 G-15 mirror/parity/FrameContractTest，UC15 仍消费 raw `im:channel:replies` characterization。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/module/trait_impl.rs`、`crates/helix-im/src/port_reply/dispatch_projection.rs`、`crates/helix-im/src/outbound/channel_admin/existing.rs`；通用 WS authority 复用 `crates/helix-im/src/channel/gate/`。
- 现有聚焦测试：`crates/helix-im/tests/g15_topic_interaction_test.rs`、`crates/helix-im/tests/uc15_16_projection_test.rs` 与 `crates/helix-im/tests/p3b_channel_outbound_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc15ChannelCreateTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
