# Source Discovery

**Feature**: `specs/021-ifv2-g-s1-cold-start-sync`
**Source Lock**: `helix=ff412d354699be64554b7764ac1b1410a4a9b154;cses-client=13e42ef2793ea5c9f39b29f5d1ce86b1aca074db;mobile=674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`
**Worktree State**: Helix Spec 生成中；cses-client clean；Mobile 保留用户既有文档与 artifact 改动，本 Feature 不覆盖。

## Facts *(mandatory)*

- **F-001**: Helix 当前 InteractionFrame 下行 schema 常量为 2，ActionInvocation 的独立 schema 1 不属于 Frame V1。 | **Evidence**: `crates/helix-im/src/interaction_frame.rs:11`
- **F-002**: `G-S1` 在 canonical Strict Matrix 中登记为 `UC-4.1 hello 全量增量`，command 为 `im_query_dialog_list`，投影为 `message.left-dialog-list:InteractionFrame/v2`。 | **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json`
- **F-003**: 当前逐 UC 文档入口为 `docs/interaction-frame-v2-全量替换-按UC/GS01/README.md`，但文档目标不能替代本次三仓运行证据。 | **Evidence**: `docs/interaction-frame-v2-全量替换-按UC/GS01/README.md`
- **F-004**: G01 已建立 Helix canonical Fixture -> PC WDIO Mock/Real -> Mobile GTest/QuickJS 的三仓基线，本 Gate 必须复用供应链规则而不是复制 G01 业务逻辑。 | **Evidence**: `docs/interaction-frame-v2-全量替换-按UC/G01/13-G01-Helix-PC-Mobile-V2验收合同.md`
- **F-005**: canonical Fixture 只能来自公开 `ImModule::handle` 的 `Effect::Emit`，平台端不得手写业务 VM。 | **Evidence**: `crates/helix-im/tests/AGENTS.md:11`
- **F-006**: MessageV4 codec 对非当前 InteractionFrame schema fail-closed，随后才允许 SurfaceRuntime 原子 apply。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:46`
- **F-007**: Mobile production event pump 把 `im:interaction:frame` 作为迁移 Surface 的唯一业务输入，legacy characterization 受显式宏隔离。 | **Evidence**: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp:2155`
- **F-008**: 当前 Strict Matrix 原始 SHA-256 为 `ef7a752be7ca77d27723a256051cc5c8d8b9cd733ce4d4b9a918e6a2880e109d`；后续 plan 必须重新锁定三仓 HEAD、Helix pin 与 Fixture manifest，不能继承本次静态快照。 | **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json`


## Unknowns

- **U-001**: `G-S1` 当前生产代码是否已覆盖全部 happy/failure/race/recovery 分支，必须在 plan 前按三仓代码和测试逐项求证。 | **Resolution**: 在 plan 的 source discovery 阶段形成 CURRENT/TARGET/GAP，不以本 Spec 的目标描述冒充完成。
- **U-002**: PC Real lane 所需 Go、账号、PG/SQLite 与 Tauri driver 前置在执行时是否可用。 | **Resolution**: 每次运行先做 bounded preflight；缺失时标记 blocked，不降级为 Mock Green。
- **U-003**: Mobile typed view 是否已暴露本 Gate 全部可观察字段。 | **Resolution**: 先验证生产 SDK/Bridge/SurfaceRuntime/QuickJS；缺字段必须回 Helix Frame 或平台只读 adapter 修复，禁止在 GTest 补 reducer。

## Decisions

- **D-001**: 本 Feature 只负责 `G-S1`，不得顺带实现下一 Gate。
- **D-002**: Helix 是业务语义和 canonical Fixture 唯一真源；PC 与 Mobile 只机械消费 Frame。
- **D-003**: 完成顺序固定为 Helix Green -> PC Mock/Real/Report Green -> Mobile GTest/QuickJS Green -> 行为/代码/Frame V2 Review Green。
- **D-004**: 本 Gate Green 前不得进入下一 Gate；Mock、accepted、HTTP 2xx 或单端测试不能提升为三仓 Green。
- **D-005**: 本 Feature 不物理删除 V1/legacy 资产；26 Gate 全部 Green 后只生成候选清单，删除范围由用户决定。
