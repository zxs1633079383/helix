# Research: G-10 切群首屏

**Source Lock**: Helix `48e8d74df519d343d99b6797b6d032644823876f`；cses-client `a0a083dd6e65051ab60d6d9bb3e044f4b3984f7a`；Mobile `3b0ff15feeb95e0d1fe89040d7e9b2ed96a3734f`；PC Helix production pin `5308d2b77174e687423c28c862fde42406408456`

## T002 Current / Target / Gap Revalidation

### Helix

- **CURRENT**: `query/local_first.rs` 已由 `im_query_messages_by_channel` 进入 local scan，按 coverage 决定 Complete 直接投影或 Partial/Miss 触发 `im_get_latest_post`；远端行在 cache persist barrier 后才发完整 `message.timeline` Frame。`TimelineProjector` 持有 scope/revision，并注册 `loadOlder`、`reply` 等 opaque ActionBinding。
- **CURRENT**: `g10_timeline_window_interaction_test.rs` 只验证 projector 的完整 `replaceView` patch、window token 和 page flags，尚未覆盖生产 `ImModule::handle`、persist failure、late/duplicate reply、query generation、recovery 或 canonical Fixture。
- **TARGET**: 共享 ScenarioRunner 必须从真实 Tick/PortReply 驱动 local Complete、remote cache、失败、竞态与 resync，捕获 `Effect::Emit` 并生成字段级 Golden。
- **GAP**: 当前没有 `g10_full_flow_test.rs`、`support/g10_scenario.rs`、`g10_wdio_fixture_contract_test.rs` 或 `fixtures/g10-wdio/`；现有 projector test 不能建立 Helix Green。

### cses-client

- **CURRENT**: MessageV4 已按 attach/snapshot/query 顺序提交 `im_query_messages_by_channel`，SurfaceRuntime 机械消费 `message.timeline`；但 G10 没有专属 Fixture、loader、Mock/real specs、PageObject 或 reporter。
- **CURRENT**: `uc-map.md` 明确 G10 为 `partial-not-green`，旧 evidence 不能证明当前 pin 的 producer、真实 bus ingress 与 DOM 因果链。
- **TARGET**: byte-exact mirror Helix canonical Fixture；Mock 只模拟 AcceptedCommand 并 replay 完整 Frame；single/dual real 从 fresh group 经真实 Tauri invoke/FrameApplied/DOM；report 分层列证据。
- **GAP**: PC 生产 pin 尚未包含待生成的 G10 canonical Fixture；必须在 Helix Green commit 后 repin，禁止从旧 raw `im:messages:query_result` 补 UI 状态。

### Mobile

- **CURRENT**: `MobileImSdk::queryMessagesByChannel` 已进入生产 `im_query_messages_by_channel` admission，`pollBridgeEvents` 只将 `im:interaction:frame` 交给 SurfaceRuntime；QuickJS command binding 已存在。
- **CURRENT**: `Uc10HistoryTest.cpp` 把 G10/G10a 混在同一旧 raw-event real scenario，等待 `im:messages:query_result` / `im:messages:older_loaded` 并断言 snapshot store；它不是 G10 Frame V2 typed-view 合同。
- **TARGET**: G10 独立 mirror/parity、production-linked `G10FrameContractTest`、fresh-group command admission、DomainEvent batch、atomic SurfaceRuntime apply 与 QuickJS `im.surface.view`。
- **GAP**: 没有 `tests/fixtures/g10/`、同步/hash gate 或 G10 Frame GTest；旧 UC 测试需保留 G10a 边界但不得以 raw event/store 作为 G10 完成证据。

## Decision 1 - 唯一业务真源

**Decision**: `G-10` 的 command（im_query_messages_by_channel）、effect（local_scan、http_read、storage_upsert、projection）、持久事实（message）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G10_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 `条件回源：`POST /api/cses/posts/getLatestPost` body `{channelId,timestamp:0}`；仅 Complete 分支不出网`；WS 为 `无`；远端表为 `post、channel`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/query/local_first.rs`；现有聚焦测试：`crates/helix-im/tests/g10_timeline_window_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc10HistoryTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
