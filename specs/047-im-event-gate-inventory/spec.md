# 功能规格：IM 事件域 Gate 与双视角合同盘点

**Feature Branch**: `main（本次未创建功能分支）`

**Created**: 2026-07-28

**Status**: Draft，等待用户确认 Gate 归类与双视角语义

**Loop Profile**: `helix-loop/v1`

**Input**: 用户要求结合当前 Helix 测试、cses-client WDIO 场景和截图中的 Gate，
先按 `im:post`、`im:channel`、`im:channel-member` 等事件域梳理指令及 sender/receiver
双视角；当前没有覆盖的行为必须显式标记，并补充“应当具备”的推导合同供人工审核。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 统一 Gate 分母与命名 (Priority: P1)

作为 Gate 评审者，我需要先知道截图、当前 Strict Matrix 和仓内测试各自包含哪些 Gate，
避免把“第一批 WDIO Gate”误称为“全量产品 Gate”。

**Why this priority**: 分母不一致会直接导致遗漏媒体、后续业务域或错误的完成率。

**Independent Test**: 对截图、Strict Matrix 和测试入口分别计数并列出差集，任何缺失 Gate
都必须有明确状态。

**Acceptance Scenarios**:

1. **Given** 截图列出 30 个 Gate，**When** 与当前 Matrix 对账，**Then** 明确截图不是
   当前 Matrix 的全部 50 个 Gate。
2. **Given** 当前 Matrix 包含 G-03，**When** 检查截图，**Then** G-03 被标为截图遗漏，
   而不是被默认为不存在或 excluded。
3. **Given** G-17 至 G-27 不在截图，**When** 输出本轮范围，**Then** 它们被标为后续范围，
   不混入本轮 30 Gate 的双视角表。

---

### User Story 2 - 评审消息类双视角 (Priority: P1)

作为 IM 产品和协议评审者，我需要对发送、接收、失败重试、撤回、转发、回应、加急、
回执和模板确认逐项区分动作发起者与其他参与者看到的终态。

**Why this priority**: 消息类操作最容易只验证 sender 本地状态，却漏掉 receiver 的 WS、
持久化、`Effect::Emit` 和可见状态。

**Independent Test**: 任取一个消息 Gate，均能回答“谁发起、谁接收、两边最终看到什么、
什么不应出现、以什么权威事实完成”。

**Acceptance Scenarios**:

1. **Given** sender 发送文本，**When** HTTP 仅被受理但尚无持久终态，**Then** sender 和
   receiver 均不得被判定为发送完成。
2. **Given** receiver 收到权威消息或消息变更，**When** 其本地提交成功，**Then** receiver
   得到自己授权后的 `{event,data}` Emit，而不是复用 sender 的 viewer-local payload。
3. **Given** sender 初次发送失败，**When** 尚未成功重试，**Then** receiver 不得看到一条
   失败占位消息。

---

### User Story 3 - 评审频道与成员类双视角 (Priority: P1)

作为群聊治理评审者，我需要区分 owner、普通成员、退出成员和其他观察者在建群、建话题、
关闭、退出、成员广播和群属性修改后的不同终态与权限。

**Why this priority**: 频道终态不是所有用户完全相同；退出、关闭、角色和成员列表尤其容易
被错误地做成全局同态。

**Independent Test**: 每个频道或成员 Gate 至少能验证一个角色差异或一个双端收敛关系。

**Acceptance Scenarios**:

1. **Given** owner 创建双人群，**When** 创建完成，**Then** owner 与受邀成员看到同一
   channel，但 viewer role 与可用动作不同。
2. **Given** 一个成员主动退出，**When** 权威成员变更提交完成，**Then** 退出者看到 left，
   留存成员仍看到 active，且 roster 不再包含退出者。
3. **Given** owner 关闭群，**When** 关闭提交完成，**Then** 所有当前成员均看到 closed。

---

### User Story 4 - 评审同步、恢复与窗口隔离 (Priority: P2)

作为多端状态恢复评审者，我需要区分“同一远端事实的双账号独立恢复”和“一个用户动作向
另一个用户传播”，并验证 timeline 导航状态不会跨用户串扰。

**Why this priority**: hello、sync、分页、定位和 anchor 大多是 persona-local 状态，不能
强行套用 sender/receiver 同步模型。

**Independent Test**: 对每个同步或 timeline Gate 标明它属于双端独立恢复、远端消息传播，
还是单端导航隔离。

**Acceptance Scenarios**:

1. **Given** 两个账号拥有独立本地状态，**When** 分别冷启动或补偿同步，**Then** 各自只
   恢复其有权看到的投影。
2. **Given** Actor 正在定位或分页，**When** Observer 保持 latest window，**Then**
   Observer 的窗口、anchor 和高亮不随 Actor 改变。
3. **Given** Observer 发送一条新消息，**When** Actor 离底浏览，**Then** Actor 收到新消息
   提示但 anchor 不跳动，直至主动跳至最新。

---

### User Story 5 - 区分代码覆盖与真实证据 (Priority: P2)

作为发布评审者，我需要看到每个 Gate 当前有哪些 Helix producer 测试和 WDIO
Mock/single/dual 场景，同时确保“文件存在”不会被写成运行通过。

**Why this priority**: 测试源码、fixture、Mock、单端 real 和双端 real 是不同证据面。

**Independent Test**: 每条覆盖声明都带证据等级；本轮未执行的运行态一律保持
`SOURCE_PRESENT / NOT_RUN`。

**Acceptance Scenarios**:

1. **Given** 某 Gate 有 dual-real 源文件，**When** 本轮没有执行它，**Then** 状态只能是
   `双端场景源码存在，未运行`。
2. **Given** Helix 有 fixture producer，**When** PC 消费该 fixture，**Then** 只能证明
   producer/consumer 合同，不证明 Go、WS、双账号或真实应用可见状态。
3. **Given** 当前场景只覆盖部分角色或字段，**When** 形成规格，**Then** 缺口必须标为
   `MISSING`，推导目标必须标为 `INFERRED`。

## Loop Contract *(mandatory)*

### Trigger

产品或工程团队准备评审截图中的第一批 WDIO Gate，或准备继续补充 sender/receiver、
角色差异、离线恢复及真实链证据。

### Goal

形成一个可人工审核的 Gate 目录：每个 Gate 有稳定事件域、触发指令、Actor/Observer
视角、权威完成条件、Emit channel、当前代码覆盖和明确缺口。

### Completion

本规格完成仅表示：

1. 截图中的 30 个 Gate 全部完成归类；
2. 当前 Matrix 与截图的差集已列明；
3. 每个 Gate 的双视角或 persona-local 性质无歧义；
4. 当前覆盖与推导补项严格分开；
5. 用户确认本轮 Gate 清单可以作为下一轮补充和计划输入。

它不表示任何 Gate runtime Green，也不表示生产代码或测试已经补齐。

### Boundaries

- 本轮只写规格、覆盖盘点和 loop discovery，不修改 Helix、PC、Mobile 或 Go 生产代码。
- 本轮不新增、重写或运行 WDIO/Cargo 测试。
- 本轮主表只覆盖截图中的 30 个 Gate；G-03 与 G-17 至 G-27 进入差集和后续决策。
- 业务出站只认 `Effect::Emit` 的 `{event,data}`；HTTP、Persist、Timer 是内部 Effect，
  Native channel 与 FFI batch 是适配器承载。
- 最多使用当前源码可证明的角色和字段；缺失项只写成推导目标。

### Human Gate

用户需要确认：

1. 截图的 30 Gate 是否就是下一阶段要维护的 WDIO 分母；
2. 是否把截图遗漏的 G-03 立即纳入同一批；
3. 本规格列出的 receiver、隐私和角色差异是否符合产品预期。

### Edge Cases

- sender 同时也是 receiver、频道只有一个成员或目标列表为空时，仍需定义是否产生远端可见
  终态，不能借用双人群断言。
- receiver 在线但没有订阅相关 channel 时，事实仍应持久化；重新查询或恢复后必须能得到
  等价 authority emit，而不是要求当时一定完成应用渲染。
- receiver 离线或错过 WS 时，后续 sync 必须恢复相同业务终态，且不能重复消息、反应、
  回执或成员。
- 同一操作有多个 receiver 时，每个 receiver 的 viewer role、权限、未读和回执可能不同；
  不允许复制 sender VM。
- HTTP 成功、command admission、WS 收帧和本地写入任一单点都不能独立成为完成条件。
- Actor 的分页、定位、高亮、anchor、overlay 和草稿属于 persona-local 状态，不向 Observer
  传播。
- 被撤回、关闭或退出等终态晚于普通消息/频道更新到达时，最终状态必须单调，旧事件不得复活。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 规格 MUST 以截图中 30 个 Gate 作为本轮审阅分母，并记录精确 Gate ID。
- **FR-002**: 规格 MUST 同时记录当前 Matrix 分母为 50、active 为 47、excluded 为 3，
  避免把截图称为全量 Matrix。
- **FR-003**: 规格 MUST 将 G-03 标记为截图遗漏但当前 Matrix active。
- **FR-004**: 规格 MUST 将 G-17 至 G-27 标记为本轮范围外，而不是已覆盖。
- **FR-005**: 每个 Gate MUST 归入 post、schedule、timeline/read、channel、
  channel-member 或 sync/recovery 之一。
- **FR-006**: 每个 Gate MUST 列出 canonical command 或 host observation。
- **FR-007**: 每个 Gate MUST 区分 Actor、Observer/Receiver 与其他角色。
- **FR-008**: 对 persona-local Gate，规格 MUST 明确写出“不传播”的状态。
- **FR-009**: 对双视角 Gate，规格 MUST 分别定义 Actor 和 Observer 的可见终态。
- **FR-010**: 对多 receiver 操作，规格 MUST 说明每个 receiver 独立授权和投影，不共享 VM。
- **FR-011**: 每个 Gate MUST 指明权威完成条件，不得使用 command admission 或 HTTP 受理代替。
- **FR-012**: 每个 durable 可见终态 MUST 在本地持久提交后才允许产生 terminal
  `Effect::Emit`。
- **FR-013**: receiver 错过在线推送后 MUST 能通过恢复路径得到同一业务终态。
- **FR-014**: sender 与 receiver 的未读、权限、角色、选中态和本地导航 MUST 独立计算。
- **FR-015**: 消息发送失败在成功重试前 MUST 只影响 sender，不得制造 receiver 消息。
- **FR-016**: 消息成功后 sender 与 receiver MUST 对账到同一 server message identity。
- **FR-017**: 撤回、reaction、加急、模板确认和回执 MUST 对现有消息做幂等更新，不新增重复行。
- **FR-018**: 定时消息创建和取消 MUST 保持创建者私有；计划真正触发后的普通消息投递需要
  独立 Gate 或明确补充。
- **FR-019**: 会话已读 MUST 只清理当前 viewer 的未读，不修改其他成员的未读。
- **FR-020**: 单条已读 MUST 更新消息作者或有权查看者的参与者回执视图。
- **FR-021**: 建群 MUST 同时定义 owner 与 invitee 的 summary、roster、role 和动作差异。
- **FR-022**: 成员退出 MUST 同时定义 leaver lifecycle 与 remaining-member roster。
- **FR-023**: 关闭群 MUST 向所有仍有权观察该频道的成员收敛 closed。
- **FR-024**: 群属性更新 MUST 向所有有权观察者同步权威字段，但动作能力按 viewer role 独立。
- **FR-025**: timeline 查询、分页与定位 MUST 保持 persona-local window、anchor、高亮和 token。
- **FR-026**: 新消息 append MUST 在 bounded window 内保持顺序、上限和稳定 anchor。
- **FR-027**: 每个 Gate MUST 标注当前 Helix 测试源与 WDIO mock/single/dual 场景源是否存在。
- **FR-028**: 覆盖状态 MUST 使用 `SOURCE_PRESENT`、`NOT_RUN`、`MISSING`、`INFERRED`
  等不会冒充运行通过的词汇。
- **FR-029**: 当前源码没有断言的 receiver 行为 MUST 标为 `MISSING`。
- **FR-030**: 基于业务不变量补出的目标行为 MUST 标为 `INFERRED`，直至用户确认。
- **FR-031**: 规格 MUST 区分 Tick、完整 Effect、`Effect::Emit`、适配器投递和真实应用证据，
  禁止互相替代。
- **FR-032**: 用户确认后的 Gate 目录 MUST 能直接作为后续 clarify/plan 的输入。

### Evidence Labels

| 标签 | 含义 |
|---|---|
| `H-FULL` | Helix 存在 production `ImModule::handle` 完整流程测试与 fixture producer |
| `H-FORMAL` | Helix 存在正式 timeline producer/fixture 测试，但没有同名 `full_flow_test` |
| `W-M/S/D` | WDIO Mock、single-real、dual-real 场景源码均存在 |
| `SOURCE_PRESENT` | 静态源码入口存在，本轮未执行 |
| `NOT_RUN` | 本轮没有运行该测试或真实链 |
| `MISSING` | 当前检查到的场景没有覆盖该断言 |
| `INFERRED` | 从领域不变量和相邻行为推导的应有合同，等待用户确认 |

## Gate Scope Reconciliation

### 本轮截图分母

截图共有 30 个 Gate：

`G-S1`、`G-S2`、`G-S3`、`G-S4`、`G-01`、`G-02`、`G-03a`、`G-04`、
`G-04a`、`G-05`、`G-06`、`G-07`、`G-08`、`G-09`、`G-10`、`G-10a`、
`G-10b`、`G-10c`、`G-10d`、`G-11`、`G-11a`、`G-12`、`G-13`、`G-14`、
`G-14a`、`G-15a`、`G-15`、`G-16`、`G-16b`、`G-16c`。

### 与当前 Matrix 的差异

| 差异 | 当前事实 | 本规格处理 |
|---|---|---|
| G-03 不在截图 | 当前 Matrix 为 active，Helix 与 WDIO 均已有媒体相关测试源 | 标记为截图遗漏，等待决定是否加入本批 |
| G-17 至 G-27 不在截图 | 当前 Matrix 仍定义后续业务 Gate，其中 G-22/G-23/G-24 excluded | 不进入本轮 30 Gate 主表 |
| Matrix epoch 漂移 | 当前 JSON 为 epoch 12、50 Gate；Roadmap 部分摘要仍写 epoch 11 | 以当前 JSON 为本轮静态真源，文档漂移另记 |
| 旧 UI 合同漂移 | 当前 Matrix 仍含 `im:interaction:frame` 投影 | 本规格不继承该投影；以 raw Emit 合同记录为待收敛目标 |

## Gate Inventory

### A. Sync / Recovery

| Gate | Trigger | Actor 视角 | Observer / 第二账号视角 | 完成 Emit | 当前覆盖与缺口 |
|---|---|---|---|---|---|
| G-S1 | hello 后全量增量初始化 | 恢复自己有权看到的 dialog，直到持久提交后 loaded | 独立恢复自己的 dialog；不是复用 Actor 快照 | `im:channel:increment`、`im:channels:loaded`；increment end 不是完成 | `H-FULL`、`W-M/S/D`、`NOT_RUN`；双账号各自 query/emit 源码存在 |
| G-S2 | `im_sync_channels` 离线 sync notify | 恢复离线期间消息、dialog 和 cursor | 独立恢复同一频道中自己有权看到的消息与 viewer-local 未读 | message/channel mutation emit + `im:sync:state` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；应补多 receiver 不同未读的字段级断言 `INFERRED` |
| G-S3 | pong gap 触发补偿 | gap owner 执行补偿，提交后恢复 runtime/dialog/timeline | 无 gap 的账号不应被 Actor 的 recovery 状态污染；若自身也有 gap 则独立补偿 | `im:sync:state`、`im:sync:too_long`；补偿 HTTP 不是完成 | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端场景覆盖独立 store 恢复 |
| G-S4 | 陌生 channel 自动补全 | hydrate channel/member/message 后恢复 summary 和 timeline | 每个账号按授权独立 hydrate，不能因 Actor 已加载而跳过 | `im:channel:increment/update`，必要时 `im:post:received` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；多角色 capability 差异为 `MISSING/INFERRED` |

### B. Post / Message Mutation

| Gate | Canonical command / observation | Sender / Actor 视角 | Receiver / Observer 视角 | 完成 Emit | 当前覆盖与缺口 |
|---|---|---|---|---|---|
| G-01 | `im_send_message` + WS `post` | sending 后只能由持久终态进入 sent/failed；自身 dialog 未读不自增 | 成功前无消息；成功后收到同一 server id 的 sent 行，dialog preview 更新，未打开时未读增加 | `im:post:sending/received/send-failed` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端场景覆盖 sender/receiver |
| G-02 | `im_send_message(type=DOCUMENT)` + WS `post` | 文档 sending/sent/failed，文档卡字段完整 | 收到同一 message id、文档标题、URL 和 preview | `im:post:sending/received/send-failed` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端断言文档字段与同一 id |
| G-03a | `im_retry_send` + WS `post` | 首次失败可重试，重用同一 temporary identity，成功后只保留一行 | 首次失败不可见；重试成功后只收到一条 sent | `im:post:sending/received/send-failed` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；receiver 失败不可见是应保留的负向合同 |
| G-04 | `im_revoke` + WS `posts_update/post_update` | 有权限者提交撤回；终态 recalled，不删除 identity | 已看到消息的 receiver 收敛 recalled；不得保留可操作正文 | `im:post:updated/deleted`，必要时 `im:channel:update` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端覆盖在线撤回 |
| G-04a | sync 中重放撤回 | 在线 Actor 已 recalled | 离线 Observer 在 sync 前仍旧态，sync 持久提交后变 recalled | `im:post:deleted/batch-updated` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端明确覆盖 online/offline 分流 |
| G-05 | `im_relay_messages` | source 不被改写；每个 target 产生独立目标消息与 dialog preview | 目标频道成员收到目标消息；不同 target 不共享 message identity | `im:posts:batch-result` + `im:post:received` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端只闭合一个 target，multi-target 原子/部分失败为 `MISSING` |
| G-06 | `im_quick_reply` + WS `post_update` | reaction 聚合更新，自己的 selected 状态按 viewer 计算 | 其他成员看到相同 reaction 计数，但 selected 状态可不同 | `im:post:updated` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；计数双端已覆盖，viewer-selected 差异需要字段级复核 `MISSING` |
| G-07 | `im_urgent_post` / `im_urgent_confirm` | requester 看到 active、pending recipients；确认后 completed | 被加急者看到 active 并可确认；确认者动作完成后双方收敛 acknowledged | `im:post:updated/readers` + `im:channel:update` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端覆盖 Observer 确认和双方收敛 |
| G-14 | `im_post_read` + WS `post_read` | Reader 对目标消息执行已读，本地不伪造他人的回执 | 消息作者/有权查看者看到 Reader 从 pending 进入 completed | `im:post:read` + `im:post:readers` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端由 Actor read、Observer receipt view 闭合 |
| G-14a | `im_template_received` + WS `post_update` | 确认者提交“已收到”，确认记录幂等 | 作者及其他观察者看到 confirmed actor 集合和计数 | `im:post:updated` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端双方状态相等 |

### C. Schedule

| Gate | Canonical command | Creator 视角 | Other member 视角 | 完成 Emit | 当前覆盖与缺口 |
|---|---|---|---|---|---|
| G-08 | `im_create_schedule` | 看到 scheduled、取消动作和重启恢复 | 创建阶段必须看不到他人的私有 schedule | `im:channel:schedule-created` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；隐私负向已覆盖；真正到点投递给 receiver 为 `MISSING` |
| G-09 | `im_cancel_schedule` | scheduled 变为无定时，取消状态关闭，重启不复活 | 始终看不到该私有 schedule 或取消细节 | `im:channel:schedule-canceled` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；双端负向已覆盖；取消与到点竞态为 `MISSING/INFERRED` |

### D. Timeline / Read Navigation

| Gate | Canonical command / observation | Actor 视角 | Observer 视角 | 完成 Emit | 当前覆盖与缺口 |
|---|---|---|---|---|---|
| G-10 | `im_query_messages_by_channel` | 得到 bounded latest window，缓存降级和失败可恢复 | 独立查询同一 authority；窗口 token、anchor 不共享 | `im:messages:query_result`，需含 latest/bounds/revision/token | `H-FULL`、`W-M/S/D`、`NOT_RUN`；窗口元数据为 `INFERRED_REQUIRED` |
| G-10a | `im_load_older_context` | 向 older 扩展窗口并保持顺序、去重和边界 | 可独立分页；Actor 分页不推动 Observer | `im:messages:query_result(mode=older)` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；older 终态字段完整性为 `MISSING` |
| G-10b | `im_load_newer_context` | 从 located/older window 向 newer 扩展 | 可独立请求 newer，导航状态隔离 | `im:messages:query_result(mode=newer)` | `H-FORMAL`、`W-M/S/D`、`NOT_RUN`；newer 终态字段完整性为 `MISSING` |
| G-10c | WS `post` append | bounded window 追加、满窗淘汰头部 | remote sender/receiver 都收敛同一新 message id，各自窗口保持上限 | `im:post:received`；淘汰后 bounds/revision emit 缺失 | `H-FORMAL`、`W-M/S/D`、`NOT_RUN`；窗口结果为 `INFERRED_REQUIRED` |
| G-10d | WS `post` while away-bottom | 离底 Actor 保持 anchor，仅增加 newer hint | Observer 发送成功；其自身可保持 latest | `im:post:received`；anchor/hint emit 缺失 | `H-FORMAL`、`W-M/S/D`、`NOT_RUN`；anchor 结果为 `INFERRED_REQUIRED` |
| G-11 | `im_get_posts` 按已知 postId 定位 | Actor 高亮目标并执行自己的滚动意图 | Observer 已有同一消息，但不得被远程高亮或滚动 | `im:read:result` / `im:messages:query_result`，定位元数据缺失 | `H-FULL`、`W-M/S/D`、`NOT_RUN`；仅 Actor 生效的定位字段为 `INFERRED_REQUIRED` |
| G-11a | `im_locate_message` 未缓存定位后 older/newer | Actor 切入 located window，双向分页仍保留 target | Observer latest window 完全不变 | `im:messages:query_result(mode=locate)`，双向 token 必填 | `H-FORMAL`、`W-M/S/D`、`NOT_RUN`；完整定位合同为 `INFERRED_REQUIRED` |
| G-12 | `im_get_replies` / `im_get_reply_branch` | 打开 thread、发送 reply、加载 branch | 查询同一 thread 后看到同一 reply id 和树关系 | `im:read:result` + `im:channel:replies` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；Observer 已打开 thread 的实时 append 为 `MISSING` |

### E. Channel / Channel-Member

| Gate | Canonical command / observation | Actor / Authority 视角 | Receiver / Other member 视角 | 完成 Emit | 当前覆盖与缺口 |
|---|---|---|---|---|---|
| G-13 | `im_channels_view` + WS `update_channel` | 当前 viewer 的 unread/mention/urgent 清零 | 其他成员的 viewer-local unread 保持不变 | `im:channel:read_echo` + `im:channel:update` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前 dual 明确验证 Observer 不变 |
| G-15a | `im_create_channel` + WS `channel_created` | creator 为 owner，看到 summary、roster 和 owner actions | invitee 为 member，看到同一 channel/roster 但 member actions | `im:channel:created` + `im:channel:members` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双端覆盖 role 差异 |
| G-15 | `im_make_topic` + WS `channel_created/post_update` | topic creator 得到 topic dialog 与 thread root | 参与者得到同一 topic dialog；有权进入后应恢复 thread | `im:channel:created` + `im:post:received/updated` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；Observer thread/roster 为 `MISSING` |
| G-16 | `im_channel_leave` / `im_channel_close` | leaver 看到 left；owner close 后看到 closed | leave 时留存成员仍 active；close 时所有成员 closed | `im:channel:left` / `im:channel:closed` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前 dual 两条分支均覆盖 |
| G-16b | WS `channel_member_update` | remaining member roster 删除 leaver | leaver 看到 left，不再作为 active roster 成员 | `im:channel:member-updated` + `im:channel:members` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；当前双人覆盖；三人以上 fanout 和 role transfer 为 `MISSING` |
| G-16c | 群名称/简介/公告修改 + WS update | 有权限 Actor 提交，看到权威版本和新字段 | Observer 收到相同权威字段，但 edit capability 按角色不同 | `im:channel:settings-updated` + `im:channel:update` | `H-FULL`、`W-M/S/D`、`NOT_RUN`；dual real 目前只覆盖 displayName，info/notice 双端为 `MISSING` |

## Derived Coverage Gaps

以下是从当前代码和场景推导出的补充候选，不算现状已完成：

1. **G-03 媒体 Gate**：截图漏项。应决定它是加入本批，还是继续单独管理 waiver。
2. **G-05 multi-target**：应覆盖两个以上 target 的独立 identity、部分失败和 receiver fanout。
3. **G-06 viewer reaction state**：应验证 Actor `selected=true`、Observer `selected=false`
   等 viewer-local 差异，而不只比较计数。
4. **G-08 schedule firing**：创建/取消只覆盖私有 schedule；到点后转为普通消息并投递 receiver
   需要独立场景。
5. **G-09 cancel race**：取消与到点投递竞态需要唯一终态，不能一边 canceled、一边又发出消息。
6. **G-10b non-empty newer**：dual real 应明确得到非空 newer page，而不只是两端独立发命令。
7. **G-12 live thread receiver**：Observer 已打开 thread 时，Actor 新回复应实时 append；
   错过 WS 后重开应恢复且不重复。
8. **G-15 Observer thread**：当前 dual real 证明双方 topic dialog，但没有证明 Observer 的
   thread detail、root 和 roster。
9. **G-16b multi-member fanout**：三人群中一人退出后，两个留存成员都应收到一致 roster；
   owner 退出时的角色转移需要产品决策。
10. **G-16c settings breadth**：displayName 已有双端场景；purpose/info、notice/header 和
    viewer capability 差异需要补齐。
11. **所有双端场景的当前状态**：源码存在不等于本轮运行通过；需要同 pin、同 epoch、
    同证据窗口重跑后才能升级。

### Key Entities

- **GateDefinition**: Gate ID、UC、事件域、触发、角色、完成条件、Emit channel 和负向约束。
- **PersonaView**: 一个 actor/viewer 对同一业务事实可见的角色、权限、未读、导航和投影。
- **AuthoritativeFact**: 服务端或本地持久层确认的消息、频道、成员、回执、schedule 或 cursor。
- **ProjectionTransaction**: 持久事实提交后交付给单个 persona 的原子可见终态。
- **CoverageClaim**: Gate、证据面、源码入口、执行状态、结果和未覆盖断言。
- **DerivedRequirement**: 当前源码未覆盖、由业务不变量推导并等待人工确认的候选合同。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 截图中的 30/30 Gate 均有事件域、触发、Actor、Observer、完成条件和覆盖状态。
- **SC-002**: 30/30 Gate 均明确属于“双端传播”“双端独立恢复”或“persona-local 不传播”。
- **SC-003**: 当前 Matrix 与截图的差集全部列出，未解释的 Gate 数为 0。
- **SC-004**: 所有覆盖声明均带证据等级，本轮误报 runtime Green 的数量为 0。
- **SC-005**: 当前场景未覆盖的行为全部使用 `MISSING`，推导目标全部使用 `INFERRED`。
- **SC-006**: 至少识别 10 个可由用户确认、删除或补充的具体覆盖缺口。
- **SC-007**: 用户能够仅根据本规格逐 Gate 判断“分类正确 / 角色错误 / 需要新增场景”，
  无需先阅读实现代码。
- **SC-008**: 规格质量清单全部通过，未保留模板占位符或未解释的 clarification。

## Assumptions

- 本轮把截图解释为“当前希望先审核的第一批 WDIO Gate”，而不是整个 Strict Matrix。
- Actor 表示动作发起者或当前被测 viewer；Observer 表示第二账号，不一定总是被动 receiver。
- 对查询、分页、定位、已读和 recovery，双视角的核心可能是隔离，而不是状态传播。
- 对建群、退出和设置变更，角色差异优先于两端 VM 字节完全一致。
- 当前 Helix 测试和 WDIO 场景源码是现状分析输入；本轮没有运行它们，因此不产生新 Green。
- `Effect::Emit` 的 `{event,data}` 是跨端业务出站真源；Native channel 与 FFI batch 仅负责
  交付，端侧不得补造第二套业务状态机。
