# Research: G-08 定时消息

**Source Lock**: Helix `4799972d96a3a0f03064587e0f091eeaf9ab72af`；cses-client `60ad2d03173b093ccb4d8e42d34d2f6b97c31909`；Mobile `9ab37d2a1137ebbfa7d14ab0f01533e1b847a5ff`；PC Helix pin `768dc9b3597523c6c40279e387d85e9f240fde8a`；Strict Matrix `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`

## Decision 1 - 唯一业务真源

**Decision**: `G-08` 的 command（im_create_schedule）、effect（http_write、storage_upsert、projection）、持久事实（channel_schedule）与 Surface（message.channel-summary）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G08_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/createSchedule` body `{post:{channelId,message,temporaryId?},schedulePostAt}``；WS 为 ``post_schedule_created``；远端表为 `post_schedule、channel、post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/outbound/posts/existing.rs`；现有聚焦测试：`crates/helix-im/tests/g08_schedule_create_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc09ScheduleTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## CURRENT / TARGET / GAP Revalidation

### Helix

- **CURRENT**: `CreateScheduleCommand` 已严格读取 `channel_id`、`message` 和正数
  `schedule_post_at`，并把可选 `type/props` 组装为
  `POST posts/createSchedule`；但它尚未拒绝未知输入字段。
  Evidence: `crates/helix-im/src/outbound/posts/existing.rs:71-120`.
- **CURRENT**: `post_schedule_created` 只向 `channel.has_schedule_post` 追加
  `PersistFire`，随后发布 legacy-named `im:channel:schedule-created` projection；没有请求已
  attach 的 `message.channel-summary` readback，也没有产出 `InteractionFrameV2`。
  Evidence: `crates/helix-im/src/ws/handlers/post_schedule_created.rs:21-56`.
- **CURRENT**: `ChannelSummaryVm` 只有群信息、生命周期、计数、通知与 actions，
  没有 schedule owner/content/status/revision 字段。
  Evidence: `crates/helix-im/src/interaction_frame.rs:1115-1134`.
- **CURRENT**: G08 focused test 仅证明 `owner_schedule_hydration` 的 owner/channel scope，
  没有 command、HTTP reply、WS authority、persist/readback、Frame、失败、竞态或恢复。
  Evidence: `crates/helix-im/tests/g08_schedule_create_interaction_test.rs:1-23`.
- **TARGET**: `post_schedule_created` 只在本地 schedule 事实和 channel flag 已进入顺序化
  持久化链后，触发 attached channel-summary readback，并由生产
  `ImModule::handle -> Effect::Emit` 发布含 owner-only schedule 字段与
  `cancelSchedule` action 的完整 Frame。
- **GAP**: 缺 typed schedule VM、durable schedule fact、attached summary refresh、
  causation/idempotency ledger、六类 ScenarioRunner、canonical Fixture 与 provenance。

### cses-client

- **CURRENT**: production UI 已有 `im_create_schedule` Tauri command、定时 overlay 和旧
  dual-account desktop scenario；新 WDIO Platform 没有 `fixtures/g08`、G08
  Mock/single-real/dual-real/report 文件。
- **CURRENT**: `submitSchedule` 在缺 Helix binding 时回退到
  `ShellService.createSchedule`，并在 Angular 内用 timeline 新 row、dialog flag transition
  或 action receipt 三分支决定 overlay 完成；这是迁移期平台状态逻辑，不满足本轮
  Helix-only terminal 合同。
  Evidence: `src/pages/message-v4/message.component.ts:240-252,290-309,711-737`.
- **CURRENT**: `ChannelSummaryVm` TypeScript contract 与 Rust 一致，尚无 schedule 字段；
  schedule dialog 本身只渲染传入 typed props，不证明数据来自 Frame。
  Evidence: `src/pages/message-v4/surface-runtime/surface-contract.types.ts:321-336` and
  `src/pages/message-v4/components/overlay/message-ui-schedule-dialog.component.ts`.
- **TARGET**: PC mirror 只读同步 Helix Fixture；Mock 只模拟 AcceptedCommand，完整 Frame 经
  bus replay；real lane 通过 production UI/Tauri/Helix/WS/FrameApplied，DOM 只观察
  Frame-owned schedule。
- **GAP**: 缺 G08 fixture supply chain、typed loader/bridge contract、WDIO page/spec/report
  和 production-boundary 证明；现有旧 scenario/artifact 不升级为当前证据。

### Mobile QuickJS

- **CURRENT**: `MobileImSdk` 与 QuickJS facade 已能 canonical dispatch
  `im_create_schedule`；`Uc09ScheduleTest.cpp` 同时覆盖 G08 创建和 G09 取消，真实链使用
  fresh `RC.createChannel`，但它只断言 command/raw projection/readback，不消费 G08
  canonical Frame。
- **CURRENT**: production `SurfaceRuntime` 已识别 `message.channel-summary`，但其字段白名单
  尚无 typed schedule；当前没有 `tests/fixtures/g08`、
  `G08FrameContractTest.cpp` 或 G08 parity 脚本。
  Evidence: `cpp/SurfaceRuntime.cpp:1696-1712` and
  `tests/gtest/uc/Uc09ScheduleTest.cpp`.
- **TARGET**: byte-exact mirror 经生产 SDK/Bridge DomainEvent batch 原子 apply，由
  QuickJS `im.surface.view` 返回与 PC 相同的 schedule typed view；默认 production build
  不可达测试注入或 raw reducer。
- **GAP**: 缺 mirror/parity、G08 Frame GTest、typed schedule decode/view 与 focused
  production-boundary/hard-cut 证据。
