# Research: G-06 快捷回复 emoji

**Source Lock**: Helix `714677ab1e5f6a5126fa45fea0a12da744cbae65`；cses-client `424d1c9e5e79351a74149c1f1c0088eb92f85352`；Mobile `1b6b77856d5d5118fce87336a85e42a959012693`

## Decision 1 - 唯一业务真源

**Decision**: `G-06` 的 command（im_quick_reply）、effect（http_write、storage_upsert、projection）、持久事实（message）与 Surface（message.timeline）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G06_WDIO_FIXTURES=1` 才更新。

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

**Decision**: HTTP 为 ``POST /api/cses/posts/quickReply` body `{postId,emoji}`；`req_id` 只透传 `Cses-Track-Id`，actor 由服务端鉴权 session 注入`；WS 为 ``post_update``；远端表为 `post、channel_event`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：`crates/helix-im/src/outbound/engagement/quick_reply.rs`；现有聚焦测试：`crates/helix-im/tests/g06_reaction_interaction_test.rs`。
- 当前 Mobile UC 测试：`/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/Uc07QuickReplyTest.cpp`。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。

## T002 Current / Target / Gap

### CURRENT

- `quick_reply.rs` 已把 canonical direct command 冻结为 `post_id + emoji`，可选
  `req_id` 只进入 transport correlation；HTTP body 仅 `{postId,emoji}`，未知
  字段 fail-closed。
- MessageV4 `react` ActionBinding 已走 `ImModule::handle` 的
  `ActionDispatch::ToggleReaction`，注册 `PendingAuthorityAction`；权威
  `post_update` 先 `PersistAtomic(message patch + cursor)`，仅 PersistOk 后 Emit
  `im:post:updated` 并 refresh attached timeline。
- 当前 focused test 只覆盖 react binding enabled 与 HTTP wire，不覆盖完整
  prerequisite、HTTP failure、WS/persist barrier、duplicate/late/recovery，也没有
  G06 canonical Frame/Fixture。
- Mobile `MobileImSdk::quickReply` 与 `Uc07QuickReplyTest` 仍要求并发送
  `user_id`；这直接违反 Strict Matrix “actor 由鉴权 session 注入、客户端不得提交
  user_id”。当前 real-chain 也因等待 raw `im:post:updated` 超时而失败，尚未以
  `im.surface.view` 作为用户结果真源。
- PC 尚无 G06 独立 Fixture parity、Mock/single-real/dual-real/report lane。

### TARGET

- 复用 G01 CreateGroupPrerequisite 创建 fresh group/source post；经 production
  ActionBinding 或 direct command 只提交 `post_id/emoji/req_id`。
- Helix 以 HTTP admission + 权威 `post_update` + PersistAtomic/PersistOk 为
  terminal authority，发布带 causation 的完整 `message.timeline`
  InteractionFrame V2；duplicate/late/failure 不产生伪成功。
- 同一 canonical Fixture byte-exact 镜像到 PC/Mobile；PC 机械 replay/real DOM，
  Mobile 通过生产 SDK/Bridge/SurfaceRuntime/QuickJS `im.surface.view` 验证
  reaction emoji、count、selected 与 correlation。

### GAP

- 新增 G06 ScenarioRunner、六类 full-flow、focused authority/persist 测试、
  Fixture producer/Golden/provenance。
- 若回放证明 direct command 没有 pending authority 或 Frame causation，需要在
  Helix 生产 handler/command 编排修复，不能在 Fixture 补写。
- PC 需要 G06 四条独立验收 lane 与 production-boundary。
- Mobile 必须删除 `QuickReplyInput.user_id` 生产输入及 JS 必填校验，并把真实链从
  raw event/readback 等待改为生产 typed view；不得新增本地 reaction reducer。
