# Research: G-14a 模板已收到

**Source Lock**: Helix `6755dab003cab2bcb6bf2e91c8133c72b270c95c`；cses-client `0d19677657d68238b85ddeba75a8736804c33ec2`；Mobile `b9cbbb3bd9c71972528f71411bab1edc93709f36`

## Current / Target / Gap

- **Current command**：`TemplateReceivedCommand` 已把公开 `postId` 归一为 `post_id`，并严格构造 `POST post/templateReceived` 的 `{postId}` wire body；HTTP success 只表示服务端受理。
- **Current authority**：权威更新来自 `post_update`。`apply_post_update` 已把 message edit 与 channel cursor 放进同一个 `PersistAtomic`，matching `PersistOk` 后才释放 render-ready `im:post:updated`；`message_item_data` 已由 `props.template.userIds` 产出 `templateReceived/templateConfirmedCount/readerIds`。
- **Current platform drift**：Mobile `Uc14TemplateReceivedTest` 仍手写 raw `im:post:updated` 并通过 `ViewModelStore` 断言字段，属于本 Gate 必须切断的第二 consumer；PC 当前也尚无 G14a canonical Fixture/Mock/Real 分层合同。
- **Target**：fresh group + template message 前置完成后，`im_template_received -> HTTP admission -> WS post_update -> PersistAtomic -> PersistOk -> attached message.timeline InteractionFrameV2`；Frame row 直接携带权威模板字段，PC/Mobile 只机械 apply。
- **Gap**：新增确定性 ScenarioRunner、完整 Frame Golden/provenance、PC Mock/Real/report、Mobile production-linked SurfaceRuntime/QuickJS contract，并把旧 raw-event/Store 测试改为 typed view；不伪造当前 Go 不提供的 participant receipt authority。

## Decision 1 - 唯一业务真源

**Decision**: `G-14a` 的 command（im_template_received）、effect（http_write、projection）、持久事实（message_receipt）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G14A_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/post/templateReceived` body `{postId}`；注意 Helix 入参读 camel `postId``；WS 为 ``post_update``；远端表为 `post、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/outbound/engagement/template_received.rs`；现有聚焦测试：`crates/helix-im/tests/g14_receipt_template_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc14TemplateReceivedTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
