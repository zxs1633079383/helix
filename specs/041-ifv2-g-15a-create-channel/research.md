# Research: G-15a 创建群聊

**Source Lock**: Helix `2c706dfdb9ca40ff3eec720287f46f9b9cb0e82e`；cses-client `5b4c5fab0d176024658379cf2f1d91830819903c`（production Helix `76da0e...`）；Mobile `bd534176ec8a7be9247a0ef986499eaf4ad2ae5f`（production Helix `f7e7ecf...`）；Strict Matrix `8b551c0d...`。

## Decision 1 - 唯一业务真源

**Decision**: `G-15a` 的 command（im_create_channel）、effect（http_write、storage_upsert、projection）、持久事实（channel、channel_member）与 Surface（message.channel-summary、message.member-roster）只由 Helix 生产路径决定。真实 command owner 是 `outbound/channel_admin/existing.rs`，terminal reconcile owner 是 `port_reply/dispatch_channel_create.rs`；不得误改只负责 `im_create_specify_owner` 的 `outbound/channel_admin/create.rs`。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G15A_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/channel/create` body `{type,userIds,displayName,forceCreate}`；session 身份、team、成员角色与默认 `pictureType=USER/picture.userIds` 由服务端派生，客户端不得提交 team/role/owner/picture`；WS 为 ``channel_created``；远端表为 `channel、channel_member、channel_members_snapshot、channel_event、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Current / Target / Gap

- **CURRENT**: canonical command casing 和 HTTP response reconcile 已存在；HTTP/WS 路径会写 channel/member，并发布 raw `im:channel:created` 或可选 dialog Frame；storage resync 已能分别生成 channel-summary/member-roster snapshot。
- **TARGET**: HTTP/WS authority 先形成单一 atomic persist correlation，matching PersistOk 后一次性发布 G15a terminal Frame；duplicate WS、迟到 HTTP 和重复 PersistOk 均不得重复 terminal write/Frame；cold attach 从 durable channel/member 恢复同字段 snapshot。
- **GAP**: 现有 create path 使用 `PersistFire` 并在回执前 emit，live path 未发布 Matrix 声明的两个 Surface，PC/Mobile 也没有 G15a canonical mirror/专属 Frame contract。

## Resolved Scope

- 当前 Helix owners：`crates/helix-im/src/outbound/channel_admin/existing.rs`、`crates/helix-im/src/port_reply/dispatch_channel_create.rs` 与 `crates/helix-im/src/ws/handlers/channel_created.rs`；现有聚焦测试：`crates/helix-im/tests/channel_create_http_reconcile_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc15ChannelCreateTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
