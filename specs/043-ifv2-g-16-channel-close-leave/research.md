# Research: G-16 关闭/退出群

**Source Lock**: Helix `509c2e2896d6b7d50528a1fceb9392efe050d0bc`；cses-client `88bdcd22169c151a321343cea73209952ec16dab`；Mobile `c5896e16b3ce3a050b5d15b925c48415c39501b0`

## Decision 1 - 唯一业务真源

**Decision**: `G-16` 的 command（im_channel_close、im_channel_leave）、effect（http_write、storage_upsert、projection）、持久事实（channel、channel_member）与 Surface（message.channel-summary）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 `ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode` 生成，普通测试比较 Golden，只有显式 `UPDATE_G16_WDIO_FIXTURES=1` 才更新。

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

**Decision**: owner 关闭使用 `POST /api/cses/channel/close` body `{channelId}` 并等待独立 `channel_close` WS authority；成员自己退出使用 `POST /api/cses/channel/member/leave` body `{channelId}`，当前生产链以 SUCCESS 回包作为本人终态 authority。两条路径都必须先提交本地生命周期事实，才能发布 Frame；剩余成员名单仍由 `channel_member_update` WS 更新。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Current / Target / Gap

- **CURRENT**: `im_channel_close` 的 wire builder 实际位于 `outbound/channel_admin/existing.rs`；`im_channel_leave` 位于 `change_dedicated.rs`。现有 HTTP wire test 只验证 endpoint/body。`channel_close` WS 当前 `PersistFire` 后立即发 raw `im:channel:closed` 与可选 dialog Frame；leave HTTP SUCCESS 同样 `PersistFire` 后立即发 raw `im:channel:left`，两者都缺 PersistOk barrier 与 canonical Frame 合同。
- **CURRENT**: Mobile `Uc16ChannelLifecycleTest.cpp` 仍以 raw `im:channel:closed/left` 驱动 `ViewModelStore` 删除 scoped rows，属于本 Gate 待移除的第二业务 reducer；QuickJS command admission 已有 close/leave 方法，但没有 G16 canonical Frame consumer 测试。
- **TARGET**: 关闭与退出各自保持独立 authority；本地生命周期 write 经可回报 Persist/PersistAtomic 成功后，Helix 才发布 `message.channel-summary` lifecycle Frame，并在已 attach 时独立发布 dialog lifecycle patch。PC/Mobile 只能消费这些 V2 Frame。
- **GAP**: 新增 G16 correlation/persist terminal、确定性 ScenarioRunner、Golden/provenance；PC 增加 fixture mirror、Mock/Real/report；Mobile 替换 raw lifecycle reducer 测试为 production-linked Frame/QuickJS typed view 合同。

- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 `NEEDS CLARIFICATION`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
