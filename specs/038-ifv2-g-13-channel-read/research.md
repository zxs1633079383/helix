# Research: G-13 会话已读

**Source Lock**: Helix `c7bf170d7aad8b2afc8185b36ab447b50bd27fa8`；cses-client `6174658be34a779ded68311848cd5c4851ec9d01`；Mobile `42819464ea4746a888a62e60f14dce18dcc27674`

## Decision 1 - 唯一业务真源

**Decision**: `G-13` 的 command（im_channels_view）、effect（http_write、storage_upsert、projection）、持久事实（channel、channel_member）与 Surface（message.left-dialog-list、message.unread-summary）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G13_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 `POST /api/cses/channels/view` body `{channels:[{id,isRoot}]}`，其 SUCCESS 只表示 command accepted；G-13 终态 authority 是定向 `update_channel` 携带的当前 viewer `channel_member` 绝对态。该绝对态本地 PersistOk 后，Helix 在同一 Frame 原子发布 `message.left-dialog-list` 与 `message.unread-summary`。`post_read` 和 `message.participant-receipts` 属 G-14，不得混入 G-13。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix command owner：`crates/helix-im/src/outbound/channel_admin/change_dedicated.rs`；WS authority：`crates/helix-im/src/ws/handlers/update_channel.rs`；paired projector：`crates/helix-im/src/dialog_frame.rs`；现有聚焦测试：`crates/helix-im/tests/g13_channel_read_interaction_test.rs`。
- 当前缺口：`update_channel` 在 `PersistFire` 前直接推进内存 projector 并 emit Frame，违反 durable terminal；必须改为相关 PersistOk 才 apply/emit。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc13ReadTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
