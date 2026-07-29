# Feature Specification: InteractionFrame V2 26 Gate 三仓逐 UC 总控

**Feature Branch**: `main (no feature branch created)`

**Created**: 2026-07-25

**Status**: Draft

**Loop Profile**: `helix-loop/v1`

**Input**: 用户要求建立一个总控 Spec 和一组独立 Gate Spec，以 G01 三仓合同为基线，先以 G02 为试点，再完成 G03a、G04..G16c 与 G-S1..G-S4；每个 UC 只有在 Helix、PC、Mobile 和独立 Review 全部 Green 后才能进入下一个。G03 文件上传及 G17-G27 进入 Gap。全部 26 Gate Green 后只生成 V1/legacy 删除候选清单，删除范围由用户决定，删除后重新执行三仓全量验收。

**Machine Index**: [gate-spec-manifest.json](gate-spec-manifest.json)

**Human Index**: [gate-spec-index.md](gate-spec-index.md)

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 用户在 PC 与 Mobile 获得一致的 IM 行为 (Priority: P1)

作为 IM 用户，我需要本轮 26 个 Gate 在桌面端和移动端表现为同一套业务行为，包括权限、排序、发送终态、失败、竞态与恢复，不因平台不同产生第二套状态机。

**Why this priority**: 跨端一致性是 Helix 的核心产品价值；任何一个 Gate 漂移都会破坏用户信任并阻止 legacy 清理。

**Independent Test**: 任取一个 Active Gate，在 fresh group 中执行完整用户行为，验证 Helix 产生权威 Frame，PC 和 Mobile 机械消费后形成一致可观察结果。

**Acceptance Scenarios**:

1. **Given** 前序 Gate 已 Green，**When** 当前 Gate 完成 Helix、PC、Mobile 与 Review，**Then** 当前 Gate 才能 Green 并解锁下一 Gate。
2. **Given** 任一 required lane 缺失或不一致，**When** 汇总状态，**Then** 当前 Gate 必须为 partial/blocked，后续 Gate 不得开始。
3. **Given** 26 Gate 全部 Green，**When**生成最终报告，**Then** 报告必须逐 Gate 列出真实证据，不以聚合数量隐藏缺面。

---

### User Story 2 - 三端维护者共享唯一业务真源 (Priority: P1)

作为 Helix、PC 与 Mobile 维护者，我需要 Helix 独占业务语义和 canonical Fixture，PC 与 Mobile 只做 transport、atomic apply 与平台渲染，防止长出三套业务 Store。

**Why this priority**: Fixture、Mock 或 GTest 中补业务字段会让测试通过而真实产品缺失，属于本轮硬阻断反模式。

**Independent Test**: 对每个 Gate 检查 production-boundary、Fixture lineage 和 consumer 代码，证明业务字段只有 Helix producer 能生成。

**Acceptance Scenarios**:

1. **Given** Helix 尚未产出某字段或状态，**When** PC/Mobile 测试需要它，**Then** 必须修复 Helix 生产链，不得在平台 Mock/GTest 增加 reducer。
2. **Given** canonical Fixture 更新，**When** 两端同步，**Then** mirror hash、schema、surface inventory 和字段语义必须与 Helix 一致。

---

### User Story 3 - 用户控制最终 legacy 删除 (Priority: P2)

作为发布决策者，我需要在全部 Active Gate Green 后看到完整 legacy/V1 删除候选清单、引用关系和风险，再决定实际删除范围；系统不得提前或擅自删除。

**Why this priority**: 文件名中的 v1 可能属于仍有效的 ActionInvocation、Matrix 或报告版本，机械全删会破坏无关合同。

**Independent Test**: 在 26 Gate 全绿前确认删除候选清单不可晋升为执行；全绿后生成分类清单并等待用户选择，删除后重新跑三仓全量验收。

**Acceptance Scenarios**:

1. **Given** 任一 Active Gate 未 Green，**When**请求 legacy 清理，**Then** 只能记录候选，不得执行物理删除。
2. **Given** 26 Gate 全部 Green，**When**用户选择删除项，**Then** 只删除获批范围并执行新的三仓全量回归。

## Loop Contract *(mandatory)*

### Trigger

G01 已形成三仓基线，但其余 Active Gate 尚未逐一证明 Helix canonical Fixture、PC Mock/Real、Mobile production/QuickJS 和独立 Review 一致。

### Goal

按固定顺序完成 26 个 Active Gate 的三仓纵向闭环，保证所有业务语义只在 Helix 实现，平台消费同一完整 InteractionFrame V2。

### Completion

26/26 Active Gate 均为 `review-green`；每 Gate required fail=0/skip=0，InteractionFrame V1 和 raw business render fallback 为 0；三仓 pin/hash/evidence 可追溯。完成后生成删除候选清单并等待用户决定，实际删除后再次获得 26/26 全绿。

### Boundaries

- Active：`G-01, G-02, G-03a, G-04, G-04a, G-05, G-06, G-07, G-08, G-09, G-10, G-10a, G-11, G-12, G-13, G-14, G-14a, G-15a, G-15, G-16, G-16b, G-16c, G-S1, G-S2, G-S3, G-S4`，共 26 个。
- Gap：`G-03, G-17, G-18, G-18a, G-18b, G-19, G-20, G-20a, G-20b, G-21, G-22, G-23, G-24, G-25, G-26, G-26a, G-26b, G-26c, G-26d, G-27`；不进入本轮 Green 分母。
- 一个 Gate Green 后才能进入下一个，不并行实现多个业务 Gate。
- 不允许 Mock/GTest/Fixture 补生产代码缺失的业务字段、排序、权限或 reducer。
- 不在用户选择前物理删除 V1/legacy 代码、文档、Fixture 或测试。
- 同一 task/validator/失败签名自动修复最多两次，第三次进入 NeedsHuman。

### Human Gate

改变范围或用户行为；改变权限/wire/数据语义；执行不可逆迁移；使用真实凭据或生产数据；决定 legacy/V1 删除范围；同一失败签名两次修复仍未收敛。

### Edge Cases

- G01 基线在最终候选 pin 上发生漂移时，必须重新 Review，不能继承旧 Green。
- G03a 失败重发是 Active Gate，不得误并入已跳过的 G03 文件上传。
- G15a 是群聊创建业务 Gate；其他 Gate 虽复用 CreateGroupPrerequisite，不能继承前一 Gate 的群数据。
- WS 与 HTTP terminal 竞态时，不得固定 Frame 次序或重复 terminal write。
- PC Real 或 Mobile production environment 缺失时，对应 Gate 必须 blocked，Mock/GTest 不能代替。
- 文件名包含 v1 但属于独立版本域时，不得自动进入删除候选。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 总控必须维护一份 26 个 Active Gate 的机器索引与人类索引，Gate ID 必须唯一且与子 Spec 一一对应。
- **FR-002**: G01 必须复用已完成的三仓基线 Feature；其余 25 个 Gate 必须各有独立 Spec Kit Feature 和 requirements checklist。
- **FR-003**: G03 必须标记为文件/媒体上传 Gap；G17-G27 的 19 个扩展 Gate 必须全部进入 Gap，不计入本轮分母。
- **FR-004**: Gate 执行顺序固定为 G01 基线、G02 试点、G03a、G04..G16c、G-S1..G-S4；前序未 Green 时后序不得开始。
- **FR-005**: 每个 canonical UC 必须创建 fresh group，复用共享 CreateGroupPrerequisite 能力，但不得复用前一个 Gate 的数据。
- **FR-006**: 每个 Gate 必须完成 Helix 六类场景、真实 producer Fixture、字段级断言、Golden 和 provenance/hash。
- **FR-007**: PC 每 Gate 必须完成 Fixture parity、Mock、Real invoke/bus、Surface apply、DOM 断言与报告；涉及 observer 的行为还必须完成双账号 real lane。
- **FR-008**: Mobile 每 Gate 必须完成 mirror parity、链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest 和 typed view；Venus 默认走共享回归。
- **FR-009**: 只有修改 Venus/QuickJS 接线能力的 Gate 才新增专属 Venus Gate；普通 Gate 不复制 Venus runner。
- **FR-010**: 每个 Gate 必须执行 production-boundary，证明真实生产入口存在且 Fixture、Mock、测试注入、characterization ABI 在默认构建不可达。
- **FR-011**: PC 与 Mobile 不得维护消息对账、排序、权限、终态、未读或 raw event 业务 reducer；缺失能力必须回到 Helix 修复。
- **FR-012**: 每个 Gate 必须经过行为、代码和 Frame V2 一致性 Review；Review 非 Green 时不得进入下一 Gate。
- **FR-013**: Review 必须核对 surface/scope/epoch/revision/mutation/directive/ActionBinding、Fixture hash、PC DOM、Mobile typed view 和终态 authority。
- **FR-014**: 每个 Gate 报告必须分别记录 Helix、Fixture、PC Mock、PC Real、Mobile Contract、Review、blocker 和最大可声明结论。
- **FR-015**: Mock、AcceptedCommand、HTTP 2xx、单端 Fixture replay、单个 DOM 截图或 GTest 通过均不得单独晋升 Gate Green。
- **FR-016**: 当前生产路径必须逐 Gate保持 Frame V2-only，V1 Frame producer/consumer 与 raw business render fallback 均为 0。
- **FR-017**: 26 Gate 全绿后只能生成带分类、反向引用和风险的删除候选清单，用户选择前不得删除。
- **FR-018**: 用户批准删除后，必须重新运行 Helix、PC 与 Mobile 的 26 Gate 全量验收；删除前证据不得冒充删除后结果。
- **FR-019**: ActionInvocation envelope、Strict Matrix schema、报告 schema 等独立 v1 版本不得因名称自动删除。
- **FR-020**: 总控必须 fail-closed：任一 Gate partial/blocked、pin/hash 漂移或 required skip>0 时，总体状态不得为 Green。

### Child Gate Specifications

- `G-02`: [发文档](../025-ifv2-g-02-document-message/spec.md)
- `G-03a`: [重发失败](../026-ifv2-g-03a-retry-failed-send/spec.md)
- `G-04`: [撤回](../027-ifv2-g-04-online-revoke/spec.md)
- `G-04a`: [离线撤回重放](../028-ifv2-g-04a-offline-revoke/spec.md)
- `G-05`: [转发/合并转发](../029-ifv2-g-05-forward-message/spec.md)
- `G-06`: [快捷回复 emoji](../030-ifv2-g-06-quick-reaction/spec.md)
- `G-07`: [加急 + 确认](../031-ifv2-g-07-urgent-message/spec.md)
- `G-08`: [定时消息](../032-ifv2-g-08-schedule-create/spec.md)
- `G-09`: [取消定时](../033-ifv2-g-09-schedule-cancel/spec.md)
- `G-10`: [切群首屏](../034-ifv2-g-10-timeline-window/spec.md)
- `G-10a`: [上拉更早历史](../035-ifv2-g-10a-load-older/spec.md)
- `G-11`: [按 postId 定位](../036-ifv2-g-11-locate-message/spec.md)
- `G-12`: [回复列表/分支](../037-ifv2-g-12-reply-thread/spec.md)
- `G-13`: [会话已读](../038-ifv2-g-13-channel-read/spec.md)
- `G-14`: [单条已读](../039-ifv2-g-14-post-read/spec.md)
- `G-14a`: [模板已收到](../040-ifv2-g-14a-template-received/spec.md)
- `G-15a`: [创建群聊](../041-ifv2-g-15a-create-channel/spec.md)
- `G-15`: [创建话题](../042-ifv2-g-15-create-topic/spec.md)
- `G-16`: [关闭/退出群](../043-ifv2-g-16-channel-close-leave/spec.md)
- `G-16b`: [member leave 广播](../044-ifv2-g-16b-member-leave-broadcast/spec.md)
- `G-16c`: [群属性修改](../045-ifv2-g-16c-channel-settings/spec.md)
- `G-S1`: [hello 全量增量](../021-ifv2-g-s1-cold-start-sync/spec.md)
- `G-S2`: [sync notify](../022-ifv2-g-s2-offline-sync-notify/spec.md)
- `G-S3`: [心跳 gap 补偿](../023-ifv2-g-s3-pong-gap-recovery/spec.md)
- `G-S4`: [陌生 channel 兜底](../024-ifv2-g-s4-unknown-channel-hydration/spec.md)

### Key Entities

- **Active Gate**: 本轮需要完成三仓纵向闭环并计入分母的独立用户行为或恢复行为。
- **Gap Gate**: 已登记当前状态和后续条件、但不进入本轮实现与 Green 分母的能力。
- **Gate Spec**: 单一 Gate 的行为、三仓责任、证据和 Review 完成合同。
- **Canonical Fixture Supply Chain**: Helix producer 到 PC/Mobile mirror 的单向、hash 固定合同。
- **UC Review Gate**: 行为、代码、Frame V2 和证据一致性的独立完成裁决。
- **Deletion Candidate Inventory**: 26 Gate 全绿后生成、等待用户选择的 legacy/V1 候选集合。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Active Gate 索引精确为 26/26；G01 基线 1 个、独立子 Spec 25 个，重复和遗漏均为 0。
- **SC-002**: Gap 索引精确包含 G03 与 G17-G27 的 19 个扩展 Gate，不计入 Active Green。
- **SC-003**: 每个 Gate 的 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 和 Review required pass rate 为 100%，fail=0、skip=0。
- **SC-004**: 26 个 Gate 的 PC/Mobile mirror 与 Helix canonical Fixture 在 schema、inventory、raw/semantic hash 上一致率为 100%。
- **SC-005**: 26 个 Gate 的 production path 中，InteractionFrame V1 producer、consumer 和 raw business render fallback 数量均为 0。
- **SC-006**: 只有 26/26 `review-green` 时总控可进入 deletion-candidate-ready；用户批准删除后新的三仓回归仍保持 26/26 Green。

## Assumptions

- G01 的已完成 Feature `019-g01-wdio-five-phases` 是方法与证据基线，不代表其他 Gate 自动完成。
- G03a 保留为失败重发 Gate；G03 文件/媒体上传单独进入 Gap。
- G17-G27 的 19 个扩展 Gate继续保留在产品 Gap Map，后续另开总控，不污染本轮分母。
- 本轮生成 Spec、checklist、discovery 与索引，不修改三仓生产代码，也不运行业务测试。
