# Research: G-04 撤回

**Source Lock**: Helix `bc03fb0d13c7062ada71467fc6ae14ea861d31ef`；cses-client `08c1843b7b7751029b6fb9b40ec54eb122f646d0`；Mobile `7bc78ef0b2510232a182b9999413bfe1a1bebe35`

## Decision 1 - 唯一业务真源

**Decision**: `G-04` 的 command（im_revoke）、effect（http_write、storage_upsert、projection）、持久事实（message）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G04_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/revoke` body `{postId}``；WS 为 ``posts_update`, `post_update`, `update_channel``；远端表为 `post、channel、channel_member、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/module/action.rs`；现有聚焦测试：`crates/helix-im/tests/g04_revoke_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc05RevokeTest.cpp`。
- `ActionDispatch::RevokeMessage` 只接受 Helix 签发的 opaque binding，把受信
  `message_id` 映射为 `im_revoke`；平台不能提交 postId 业务 payload。
- `im_revoke` 只产生 `POST /api/cses/posts/revoke {postId}`；HTTP Ok 只保留等待
  WS authority 的 pending action，HTTP Err 只终止该请求，均不产生撤回成功 Frame。
- WS `posts_update` 对 revoked post 构造 message revoke op、cursor 和 viewer-ready
  event 的单个 `PersistAtomic`；只有匹配 `PersistOk` 后才提交 cursor、释放事件并
  以同一 requestId 刷新 attached timeline。
- timeline 从 durable `revoke/revoked=true` 投影 `content=Recalled`、
  `delivery=Recalled` 且清空 actions/quickActions/reply/reaction/topic；恢复 snapshot
  复用同一 projector，不由平台替换撤回文案或生命周期。
- 当前缺口集中在完整六类 ScenarioRunner、canonical Fixture/Golden/provenance、
  PC 独立 Mock/Real/report 和 Mobile production-linked Frame 合同，不需要新建第二套
  生产 reducer。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
