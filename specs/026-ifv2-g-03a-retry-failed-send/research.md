# Research: G-03a 重发失败

**Source Lock**: Helix `918999d649007d92b085d8f5aba73329c9241ee4`；cses-client `13e42ef2793ea5c9f39b29f5d1ce86b1aca074db`；Mobile `674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`

## Decision 1 - 唯一业务真源

**Decision**: `G-03a` 的 command（im_retry_send）、effect（storage_read、http_write、projection）、持久事实（message、pending_send）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G03A_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/create``；WS 为 ``post` 回声；失败为本地 emit`；远端表为 `post、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/send/retry_send.rs`；现有聚焦测试：`crates/helix-im/tests/send_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc04RetrySendTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## T002 CURRENT / TARGET / GAP Revalidation

### Helix

- **CURRENT**：`im_retry_send{message_id}` 先查内存 `pending_sends`；冷启动依次按
  `temporary_id`、server `id` 查询 durable `message`。failed/unsend 才可 retry；
  sent 幂等忽略；原 `temporaryId`、body、reply anchors、createAt 与 request trace
  被复用，不分配第二条消息身份。
- **CURRENT**：现有 `send_test.rs` 已覆盖 cached body、reply anchors、native restart
  rehydrate、server-id fallback 与 late P1 后仍可 retry。
- **GAP**：`continue_retry` 当前用 `PersistFire(sending)` 后立即 Emit/HTTP，没有
  `PersistOk` barrier；尚无 G03a 六类 ScenarioRunner、InteractionFrameV2 字段合同或
  canonical Fixture。该顺序不能直接满足本 Spec 的 durable-before-visible 要求。
- **TARGET**：通过有 correlation 的 terminal persist barrier 推进 sending Frame/HTTP，
  并以 HTTP failure、WS-first/late HTTP、duplicate inbound、cold recovery 证明同一
  temporary identity 的单一 terminal authority。

### PC

- **CURRENT**：MessageV4 `retry` Action 只在 `sendStatus=failed` 且 Helix Action
  存在时显示；`ShellService` 仅发送 `im_retry_send{message_id}`，不重建原 payload。
  Tauri command 已注册，production command port 已存在。
- **GAP**：没有 G03a Fixture mirror、typed loader、Mock lane、PageObject、
  single/dual real 或 reporter。当前真实 harness 也没有通用的“文本发送失败一次后恢复”
  可观察能力；不得用 Mock 或手写 failed VM 替代。
- **TARGET**：失败 Frame 先由 Helix producer 建立；Mock 只 replay。Real lane 必须通过
  test-only 且 production-unreachable 的 transport failure control，或现有真实可证伪
  失败机制，先得到同一 fresh group 的 failed row，再恢复链路执行真实 retry。

### Mobile

- **CURRENT**：生产 `MobileImSdk::retrySend`、QuickJS `im.posts.retrySend` 与 Venus
  `ImPostsApi` 已机械转发 `messageId -> im_retry_send`；不构造原消息 body。
- **CURRENT**：旧 `Uc04RetrySendTest` 的 real case 通过缺失本地 FILE 制造 media
  failure，并使用 characterization-only `im.store`/raw events 验证恢复。
- **GAP**：该旧 case 进入 G03 文件上传/media retry 路径，与 Strict Matrix G03a
  `storage_read -> http_write -> message.timeline` 不同，不能作为 G03a V2 Contract。
  当前也没有 G03A canonical mirror、SurfaceRuntime/QuickJS typed view 或 C ABI 字段合同。
- **TARGET**：新增 production-linked G03A Frame GTest，验证同一 temporary identity 从
  failed -> retry sending -> sent、schema/revision fail-closed 与 typed `im.surface.view`；
  不把 media reducer、`im.store` 或 raw event 带回生产合同。
