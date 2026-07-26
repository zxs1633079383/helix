# Source Discovery

**Feature**: `specs/041-ifv2-g-15a-create-channel`
**Source Lock**: `helix=2c706dfdb9ca40ff3eec720287f46f9b9cb0e82e;cses-client=5b4c5fab0d176024658379cf2f1d91830819903c;pc-production-helix=76da0e898db39e2f343d36790ad2ccfbb92bf154;mobile=bd534176ec8a7be9247a0ef986499eaf4ad2ae5f;mobile-production-helix=f7e7ecf041427559e23ca96a1e982eb8a607e94b`
**Worktree State**: Helix 只有本 Feature 的 `SESSION.md` 与 loop ledger；cses-client clean；Mobile 只保留用户既有未跟踪 bundle 与 `tests/.DS_Store`，本 Feature 不覆盖。

## Facts *(mandatory)*

- **F-001**: Helix 当前 InteractionFrame 下行 schema 常量为 2，ActionInvocation 的独立 schema 1 不属于 Frame V1。 | **Evidence**: `crates/helix-im/src/interaction_frame.rs:11`
- **F-002**: `G-15a` 在 canonical Strict Matrix 中登记为 `UC-5.1 创建群聊`，command 为 `im_create_channel`，投影为 `message.channel-summary:InteractionFrame/v2、message.member-roster:InteractionFrame/v2`。 | **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json`
- **F-003**: 当前逐 UC 文档入口为 `docs/interaction-frame-v2-全量替换-按UC/G15/README.md`，但文档目标不能替代本次三仓运行证据。 | **Evidence**: `docs/interaction-frame-v2-全量替换-按UC/G15/README.md`
- **F-004**: G01 已建立 Helix canonical Fixture -> PC WDIO Mock/Real -> Mobile GTest/QuickJS 的三仓基线，本 Gate 必须复用供应链规则而不是复制 G01 业务逻辑。 | **Evidence**: `docs/interaction-frame-v2-全量替换-按UC/G01/13-G01-Helix-PC-Mobile-V2验收合同.md`
- **F-005**: canonical Fixture 只能来自公开 `ImModule::handle` 的 `Effect::Emit`，平台端不得手写业务 VM。 | **Evidence**: `crates/helix-im/tests/AGENTS.md:11`
- **F-006**: MessageV4 codec 对非当前 InteractionFrame schema fail-closed，随后才允许 SurfaceRuntime 原子 apply。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:46`
- **F-007**: Mobile production event pump 把 `im:interaction:frame` 作为迁移 Surface 的唯一业务输入，legacy characterization 受显式宏隔离。 | **Evidence**: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp:2155`
- **F-008**: 当前 Strict Matrix 原始 SHA-256 为 `8b551c0d7aead1e44bbba355f65d65f17c883d84fbb2090854f0942bb4904fb7`；PC production pin 为 `76da0e...`，Mobile production pin 为 `f7e7ecf...`。 | **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json`、`/System/Volumes/Data/workspace/angular/cses-client/src-tauri/Cargo.toml:89`、`/System/Volumes/Data/workspace/c/mobile-qucik-c++/contract/helix-source.lock.json`
- **F-009**: `im_create_channel` 的真实 command owner 是 `outbound/channel_admin/existing.rs`，HTTP terminal reconcile 在 `port_reply/dispatch_channel_create.rs`；`outbound/channel_admin/create.rs` 只处理不同命令 `im_create_specify_owner`。 | **Evidence**: `crates/helix-im/src/outbound/channel_admin/existing.rs:48`、`crates/helix-im/src/port_reply/dispatch_channel_create.rs:7`
- **F-010**: 当前 HTTP/WS create path 以 `PersistFire` 写 channel/member 后立即发布 raw `im:channel:created`，只在已挂载 dialog 时产生 dialog Frame；尚未经过 matching PersistOk 释放 `message.channel-summary` 与 `message.member-roster` 的 live terminal Frame。 | **Evidence**: `crates/helix-im/src/port_reply/dispatch_channel_create.rs`、`crates/helix-im/src/ws/handlers/channel_created.rs`、`crates/helix-im/src/ws/handlers/channel_member_update.rs`


## Unknowns

- **U-001**: `G-15a` 当前生产代码尚未覆盖 persist barrier、live channel-summary/member-roster、duplicate/late terminal 与 cold recovery 的同一 ScenarioRunner。 | **Resolution**: 先在 Helix 补确定性 RED 测试和唯一 terminal correlation，再生成 Fixture；平台不得补偿。
- **U-002**: PC Real lane 所需 Go、账号、PG/SQLite 与 Tauri driver 前置在执行时是否可用。 | **Resolution**: 每次运行先做 bounded preflight；缺失时标记 blocked，不降级为 Mock Green。
- **U-003**: Mobile typed view 是否已暴露本 Gate 全部可观察字段。 | **Resolution**: 先验证生产 SDK/Bridge/SurfaceRuntime/QuickJS；缺字段必须回 Helix Frame 或平台只读 adapter 修复，禁止在 GTest 补 reducer。

## Decisions

- **D-001**: 本 Feature 只负责 `G-15a`，不得顺带实现下一 Gate。
- **D-002**: Helix 是业务语义和 canonical Fixture 唯一真源；PC 与 Mobile 只机械消费 Frame。
- **D-003**: 完成顺序固定为 Helix Green -> PC Mock/Real/Report Green -> Mobile GTest/QuickJS Green -> 行为/代码/Frame V2 Review Green。
- **D-004**: 本 Gate Green 前不得进入下一 Gate；Mock、accepted、HTTP 2xx 或单端测试不能提升为三仓 Green。
- **D-005**: 本 Feature 不物理删除 V1/legacy 资产；26 Gate 全部 Green 后只生成候选清单，删除范围由用户决定。
