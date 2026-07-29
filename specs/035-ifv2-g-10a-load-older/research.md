# Research: G-10a 上拉更早历史

**Source Lock**: Helix `4f398cc05fcdd9346d5d8163ab6110a72be6b949`；cses-client `4105aefaf5c470a77c52a23318edae57cdff3b70`；Mobile `0e9400c5044d19dd47f7b97899ff2a611ca57a92`

## Decision 1 - 唯一业务真源

**Decision**: `G-10a` 的 command（im_load_older_context）、effect（http_read、storage_upsert、projection）、持久事实（message）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G10A_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/postContext` body `{postId,before}``；WS 为 `无直接 WS`；远端表为 `post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/channel/older_context.rs`；现有聚焦测试：`crates/helix-im/tests/g10_timeline_window_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc10HistoryTest.cpp`。

## T002 CURRENT / TARGET / GAP（2026-07-25）

- **CURRENT Helix**：`im_load_older_context` 已有最多 8 轮 `posts/postContext` 状态机，能做严格更早过滤、跨批去重、pivot 推进与 malformed response fail-closed；终点仍直接 emit raw `im:messages:older_loaded`，未经过 message Persist barrier，也未生成 `message.timeline` Frame V2。
- **CURRENT PC**：MessageV4 `ShellService` 在平台 Map 中保存目标 window size，监听 raw `im:messages:older_loaded` 后再发 `im_query_messages_by_channel`。这是平台侧二次编排；ProjectionBus/registry 仍把 raw event 注册为业务 consumer。
- **CURRENT Mobile**：生产 `MobileImSdk::loadOlderContext` 与 QuickJS `im.posts.loadOlderContext` admission 已存在，且 canonical payload 会丢弃 legacy 字段；UC10 测试只把该能力标为 future G10a，尚未验证 production DomainEvent batch、SurfaceRuntime atomic apply 与 typed view。
- **TARGET**：Helix 在 HTTP authority 后把可见旧消息原子写入 message storage，Persist 成功才以同一 attached timeline scope 发布完整 replaceView Frame patch；HTTP/Persist 失败发布保持既有 items 的 failed Frame，重复或迟到 PortReply 不追加 revision。PC/Mobile 只消费完整 Frame。
- **GAP**：缺少 `LoadOlderContext` 的 persist continuation、attached-window prepend projector、Frame producer Scenario/Fixture；PC 需删除 raw event 与 pending Map；Mobile 需把 G10a 纳入 production-linked GTest/real runner。Observer 没有独立 WS 事实，双端只验证共享频道权威历史的各自主动查询，不伪造推送 Frame。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
