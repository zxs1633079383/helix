# Research: G-05 转发/合并转发

**Source Lock**: Helix `8f3a14d81ffcb9d7d90224d2838fe3d623eeeb0e`；cses-client `f0cc3d187f4d84165a485d21abe91175264519f4`；Mobile `883fca7f744feb6d38c48ff6b607fe7818c42bf3`

## CURRENT - 生产链与已有证据

- Helix `crates/helix-im/src/forward.rs:27-116` 用有界
  `PendingForwardDeliveryLedger` 保存 request-to-target authority；HTTP 只裁剪
  accepted targets，只有可信 WS post 经 durable dialog readback 后才完成 target。
- Helix `crates/helix-im/src/forward.rs:181-283` 严格接收
  `post_ids/target_channel_ids/mode/req_id`，逐条读取本地 message row，再由
  `forward_shape.rs` 装配 production Post 并调用 `im_create_posts`。
- Helix `crates/helix-im/src/ws/handlers/post.rs:85-93` 只信任顶层
  `envelope.tracing.csesTrackId`；`port_reply/dispatch.rs:126-173` 在 channel
  readback 成功并实际发出 terminal Frame 后消费 authority ledger。
- Helix `crates/helix-im/tests/g05_forward_interaction_test.rs:11-411` 已覆盖 opaque
  forward action、per-target partial admission、伪造 props 拒绝与 Persist barrier，
  但没有可复用完整 ScenarioRunner、恢复/重复/迟到完整矩阵及 canonical Fixture。
- PC production 已在 `src-tauri/src/features/helix_im/commands.rs` 注册
  `im_relay_messages`，但当前没有 G05 fixture loader、WDIO Mock/single-real/
  dual-real、专属 PageObject 或 report。
- Mobile production 已由 `MobileImSdk.cpp:1426-1440` admission
  `im_relay_messages`，经 `MobileFfiBridge.cpp:212-224` 批量 DomainEvent 边界、
  `SurfaceRuntime.cpp:2112` typed apply 与 QuickJS `im.surface.view` 暴露。
  `Uc06RelayTest.cpp:18-42` 已创建 source/target fresh group，但 real-chain 仍以
  raw `im:post:received` 和 HTTP readback 作为完成证据，没有断言 canonical
  Frame 与 QuickJS typed view。

## TARGET - G05 纵向闭环

- 一个 `G05ScenarioRunner` 仅通过 `ImModule::handle` 驱动
  Command/PortReply/Inbound，覆盖 source lookup、HTTP partial/business/transport
  failure、可信 WS authority、Persist failure replay、duplicate/late reply 与 cold
  recovery。
- canonical Fixture 必须来自实际 `Effect::Emit` 解码，至少区分 source
  revocable timeline、HTTP accepted 非终态、WS target timeline 与
  left-dialog terminal，不把两条独立 correlation 合并成固定帧序。
- PC 只做 AcceptedCommand 与完整 Frame replay 的机械消费；Real lane 必须从 UI
  fresh source/target group 经真实 Tauri/Helix/HTTP/WS/FrameApplied 到 DOM。
- Mobile GTest 链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding，并以
  `im.surface.view` typed timeline/dialog 作为最终证据；不得在测试或 JS 增加转发
  reducer。

## GAP - 本 Feature 必须关闭

- 缺少 `g05_full_flow_test.rs`、`support/g05_scenario.rs`、
  `g05_wdio_fixture_contract_test.rs` 和 `fixtures/g05-wdio/`。
- 缺少 PC G05 mirror/parity/loader/Mock/Real/PageObject/reporter 与生产边界登记。
- 缺少 Mobile G05 fixture parity、完整 Frame contract、typed real-chain 断言与
  source-pin/legacy hard-cut 的 G05 登记。
- 现有 HTTP acceptance 路径可能刷新 attached timeline，但它不是 delivery
  terminal；新测试必须防止把 admission、raw WS 或远端 readback单独升级为 Green。

## Decision 1 - 唯一业务真源

**Decision**: `G-05` 的 command（im_relay_messages）、effect（storage_read、http_write、projection）、持久事实（message、channel）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G05_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/createPosts` body `{posts:[Post...],channelIds:[...]}``；WS 为 ``post` per target channel`；远端表为 `post、channel、channel_member、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/forward.rs`；现有聚焦测试：`crates/helix-im/tests/g05_forward_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc06RelayTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
