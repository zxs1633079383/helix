# Research: G-14 单条已读

**Source Lock**: Helix `d381e42eae0955b5231a55b54fa3eade1ddb81ca`；cses-client `e67384f9a1c78bb2da3aafe4ef4206563d231aa4`；Mobile `fe0fa86a3bc27bf3ba416c3c7f07c89b6c80ecbb`

## CURRENT / TARGET / GAP

### CURRENT

- `crates/helix-im/src/outbound/posts/existing.rs` 已把 `im_post_read` 映射到 `POST /api/cses/post/read`，并能透传合法非空的 `posts` 字符串数组。
- `crates/helix-im/src/ws/handlers/post_read.rs` 已解析无 `event_seq` 的 `post_read` 权威帧并生成 `message.read_bits` 覆盖写，但当前使用 `PersistFire`，同时立即发布 raw `im:post:read` 并请求 timeline readback；持久化失败无法阻止终态投影。
- `crates/helix-im/src/surface_runtime.rs` 已能从 `im_post_read_list` HTTP 权威回包生成 `message.participant-receipts` typed snapshot，但在线 `post_read` 并不会在持久化成功后刷新已 attach 的 receipt slot。
- `crates/helix-im/tests/g14_receipt_template_interaction_test.rs` 只冻结 wire 与 projector 的局部行为；当前没有 G14 production full-flow、canonical Fixture 或 Golden。
- cses-client 已有通用 `message.participant-receipts` codec、SurfaceRuntime 与组件，但没有 G14 Fixture、Mock/Real、PageObject 或 report。
- Mobile 已有通用 Frame V2 production SDK/Bridge/SurfaceRuntime/QuickJS typed view；`Uc13ReadTest.cpp` 只属于 G13 频道级已读，当前没有独立 G14 单条已读测试或 Fixture mirror。

### TARGET

- `post_read` 权威事实必须改为相关 `Persist`，只有 matching `PersistOk` 才发布 raw 兼容事件、刷新 attached timeline，并对已 attach 的 read receipt slot发起 `post/read/list` 权威重取。
- G14 ScenarioRunner 从 fresh group、receipt attach、`im_post_read` admission、WS authority、PersistOk、receipt authority reply生成完整 `message.participant-receipts` Frame；persist failure、duplicate/late reply 与 recovery 均 fail-closed。
- Helix canonical Fixture 是 PC/Mobile 唯一合同源；PC/Mobile 只做动态身份重绑定、原子 apply 与 typed view/DOM 断言。
- Mobile 使用独立 `Uc14PostReadTest.cpp`，不把 G14 单条回执语义并入 G13 的频道未读清零测试。

### GAP

- 缺少 `PostReadPersist` correlation 与 matching PortReply terminal 编排。
- 缺少已 attach read receipt slot 的 authority 后刷新能力。
- 缺少 G14 full-flow、字段级 Frame、Fixture/provenance 和三仓 consumer 验收。
- PC/Mobile production 通用 V2 consumer 已存在；除非测试证明真实缺口，不新增业务 Store、raw reducer 或测试注入 API。

## Decision 1 - 唯一业务真源

**Decision**: `G-14` 的 command（im_post_read）、effect（http_write、storage_upsert、projection）、持久事实（message.read_bits）与 Surface（message.participant-receipts）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G14_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/post/read` body `{channelId,posts:[postId]}``；WS 为 ``post_read``；远端表为 `post、channel_member、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix outbound owner：`crates/helix-im/src/outbound/posts/existing.rs`；WS authority owner：`crates/helix-im/src/ws/handlers/post_read.rs`；现有聚焦测试：`crates/helix-im/tests/g14_receipt_template_interaction_test.rs`。
- 当前 Mobile `Uc13ReadTest.cpp` 只覆盖 G13；G14 必须建立独立 `Uc14PostReadTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
