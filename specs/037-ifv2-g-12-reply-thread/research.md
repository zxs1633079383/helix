# Research: G-12 回复列表/分支

**Source Lock**: Helix `533f1b654351f6708e34344aa509fcd58b81cf86`；cses-client `509d10cae61e66f6d0a90461daa8d1ad192ba1b8`；Mobile `49b94309710c511c1ffae1bf41f6a55f022fabe2`

## Current / Target / Gap

### Current

- Helix direct `im_get_replies` HTTP success still emits raw `im:read:result` and
  `im:channel:replies`; it does not emit `message.thread-detail`. Direct
  `im_get_reply_branch` only emits a typed patch when a thread snapshot is already
  attached. | **Evidence**: `crates/helix-im/src/port_reply/dispatch.rs:618`、
  `crates/helix-im/src/port_reply_emit.rs:73`
- `__helix_request_surface_snapshot` can already obtain the authority page and emit
  a complete typed `threadDetailSnapshot`; the projector validates channel/root,
  revision and bounded node count. | **Evidence**:
  `crates/helix-im/src/surface_runtime.rs:329`、
  `crates/helix-im/src/projector/thread_detail.rs:58`
- PC `loadReplies` currently sends the direct query and then issues a second
  Surface snapshot request; `loadReplyBranch` only sends the direct branch query.
  Angular already reads `ThreadDetailVm` from SurfaceRuntime and has no G12 raw
  reducer. | **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/shell.service.ts:185`
- Mobile production still exposes and reduces raw `im:channel:replies` into
  `reply_tree_row`/`reply_row`. Its UC12 real chain waits on that raw event and the
  shared `remoteReadback` waits on the retired `im:messages:query_result`, which is
  the observed timeout boundary. | **Evidence**:
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp:2602`、
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc12RepliesTest.cpp:48`、
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/support/UcJsContractFixture.hpp:813`
- G12 currently has one focused projection normalization test, but no six-scenario
  runner, canonical Fixture, provenance or cross-repository G12 mirror. | **Evidence**:
  `crates/helix-im/tests/g12_reply_thread_interaction_test.rs:1`

### Target

- Both direct reply commands terminate only through a complete
  `message.thread-detail` Frame: snapshot for `im_get_replies`, patch for an
  attached `im_get_reply_branch`; detached branch remains view-silent and is
  recoverable by a later authoritative snapshot.
- Authority mismatch, malformed envelope, HTTP failure, duplicate/late reply and
  revision mismatch produce no partial Frame and do not mutate the projector.
- The read-only query does not invent a persistence barrier: HTTP authority is
  sufficient for the view, while any future durable cache write must add and close
  its own matching `PersistOk`.
- PC removes the redundant second business query once the direct command emits the
  snapshot. Mobile removes G12 raw event admission/reducer/waits and observes only
  production Frame batches through `SurfaceRuntime` and QuickJS typed view.

### Gap

- Refactor the existing thread snapshot/branch composer into one production helper
  used by direct reply PortReply and Surface resync without duplicating business
  parsing or ordering.
- Remove G12 raw producer/consumer evidence (`im:channel:replies`,
  `im:read:result` for these commands) from required paths and replace its tests
  with Frame V2 field/correlation assertions.
- Add the missing scenario runner, canonical Fixture/provenance, PC
  Mock/Real/report supply chain and Mobile Frame contract/parity suite.
- Replace Mobile UC12 raw waits and legacy remote readback with correlation against
  `im:interaction:frame` plus typed `message.thread-detail`/timeline observations.

## Decision 1 - 唯一业务真源

**Decision**: `G-12` 的 command（im_get_replies、im_get_reply_branch）、effect（http_read、projection）、持久事实（message、reply_tree）与 Surface（message.thread-detail）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G12_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/getReplies` body `{replyId,pageNumber,pageSize,revoke?}`；`POST /api/cses/posts/getReplyBranch` body `{replyFirstLevelId,pageNumber,pageSize,revoke?}``；WS 为 `无直接 WS`；远端表为 `post`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/query/read_relay.rs`；现有聚焦测试：`crates/helix-im/tests/g12_reply_thread_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc12RepliesTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
