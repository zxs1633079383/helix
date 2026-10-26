# Feature Specification: MessageV4 Timeline V3 增量窗口

**Feature Branch**: `046-timeline-v3-delta-window`

**Created**: 2026-07-28

**Status**: Draft

**Loop Profile**: `helix-loop/v1`

**Input**: User description: "将 cses-client 的 `20260727-214840-timeline-v3-delta`
规格迁入 Helix，并按当前 Spec Kit 格式建立最新 Feature Spec。目标是将 MessageV4 Timeline
从完整视图替换升级为版本化原子增量，覆盖首屏、实时消息、发送对账、上拉、下拉、定位、冷缓存恢复
和 V2 下线。"

**Product Requirement**: [MessageV4 Timeline V3 增量窗口 PRD](../../docs/2026-07-27-message-v4-timeline-v3-prd.md)

> 本规格定义用户可观察行为、合同边界和验收结果。输入规格、关联 PRD、当前 Helix 合同与实现事实
> 的固定版本和引用记录在 `discovery.md`；发生冲突时必须先修订权威合同和本规格，不得在实现阶段
> 自行选择解释。

## User Scenarios & Testing *(mandatory)*

### 用户故事 1：打开频道并看到最新消息（优先级：P1）

作为已登录用户，我首次进入频道或重新进入已淘汰缓存的频道时，可以按前端窗口配置看到客户端
持久化事实中的最新消息，并明确知道是否还能加载更早或更新的消息；未配置时默认 20 条，合法配置
可以扩大到 60 条。

**为什么优先**：首屏是所有 Timeline 行为的基础。窗口内容、顺序或边界不准确时，后续实时更新、
分页和定位均无法可信运行。

**独立验收**：为一个频道准备超过 60 条有稳定复合身份的持久化消息，分别使用默认 20 和配置 60
首次挂载 Timeline，验证展示数量、顺序、去重和更早边界均与真实持久化结果一致。

**验收场景**：

1. **给定** 频道持久化了 21 条以上消息，**当** 用户首次进入频道，**那么** 系统以完整 Snapshot
   展示最新 20 条，并声明存在更早消息。
2. **给定** 频道不足 20 条消息，**当** 用户首次进入频道，**那么** 系统展示全部消息，并声明没有
   更早消息。
3. **给定** 一个冷频道的消息实体已从 UI Cache 淘汰，**当** 用户重新进入，**那么** 系统从当前
   持久化事实重新建立最新 20 条窗口，不复用陈旧窗口。
4. **给定** 多条消息拥有相同创建时间，**当** 建立最新窗口，**那么** 每条消息仍具有确定顺序，
   且不会遗漏或重复。
5. **给定** 前端未提供窗口数量，**当** 首次挂载、上拉、下拉或定位，**那么** 系统统一使用默认
   `pageSize=20`。
6. **给定** 前端提供合法 `pageSize=60`，**当** 执行同一组操作，**那么** 系统使用相同算法返回
   至多 60 条，而不是切换到另一套分页或排序规则。
7. **给定** 前端提供 `pageSize=0` 或大于 60，**当** 请求窗口，**那么** 系统 fail closed，且不发起
   无界查询或部分提交。

---

### 用户故事 2：实时消息只产生必要变化（优先级：P1）

作为正在阅读频道的用户，我发送、收到或更新一条消息时，只看到对应消息和必要 Range 发生变化，
不会因为完整 Timeline 被替换而跳屏、重复消息或丢失阅读位置。

**为什么优先**：实时消息是最高频路径。每次完整替换窗口会放大数据处理、渲染和视觉稳定风险。

**独立验收**：在一个已有 20 条消息的窗口中分别执行发送、服务端终态、接收、撤回、回复和 Reaction，
验证每次只改变目标实体及必要位置，并保持其他消息实体和可见行身份。

**验收场景**：

1. **给定** 用户位于 Timeline 底部，**当** 收到新消息，**那么** 新实体增量加入最新 Range，窗口
   自动跟随到底部。
2. **给定** 用户已离开底部并固定阅读位置，**当** 收到新消息，**那么** 数据和最新水位更新，当前
   视觉 anchor 不动，只出现新消息提示。
3. **给定** 用户发送一条处于 sending 的消息，**当** 服务端终态到达，**那么** 同一个稳定实体更新为
   sent 或 failed，不产生第二条消息，也不重建原可见消息行。
4. **给定** 一条消息被撤回、被回复或 Reaction 变化，**当** 权威更新到达，**那么** 系统只 Upsert
   该消息；撤回不被解释为物理删除。
5. **给定** 目标消息在 Cache 内但不在 RenderWindow，**当** 它被更新，**那么** Cache 中实体变化，
   当前渲染窗口不产生无关可见变化。
6. **给定** 目标消息已经淘汰，**当** 权威更新到达，**那么** 持久化事实更新；未来重新加载时显示
   最新状态，而不强制恢复完整 UI Chunk。

---

### 用户故事 3：连续上拉、下拉而不跳动（优先级：P1）

作为阅读历史消息的用户，我可以连续上拉更早消息，也可以从定位窗口向下加载更新消息，现有可见
消息保持原位置，直到我主动选择返回最新。

**为什么优先**：分页和虚拟滚动必须共同维护视觉连续性，否则用户会丢失正在阅读的上下文。

**独立验收**：从一个中间窗口开始，多次交替加载更早和更新数据，验证每页无重复、无遗漏、边界最终
耗尽，并且加载前的第一条可见消息保持相同视觉偏移。

**验收场景**：

1. **给定** 当前 Range 之前存在更早消息，**当** 用户上拉触顶，**那么** 系统按当前 `pageSize`
   加载至多对应数量的更早消息并扩展 Range，原视觉 anchor 保持不变。
2. **给定** 当前定位 Range 之后存在更新消息，**当** 用户下拉触底，**那么** 系统加载下一页并更新
   更新方向边界，不自动切到频道底部。
3. **给定** 分页结果与已有 Range 重叠，**当** 应用结果，**那么** 重复实体被去重，相邻或重叠 Range
   在可验证时合并。
4. **给定** 一个方向已无更多持久化消息，**当** 用户再次触达该边界，**那么** 系统明确返回耗尽，
   不重复请求相同页。
5. **给定** 一个较早分页响应在用户完成其他导航后才到达，**当** 该响应有效，**那么** 它可以补充
   Cache，但不得抢走当前窗口。

---

### 用户故事 4：定位消息后继续双向阅读（优先级：P1）

作为通过搜索、回复引用、未读起点或其他入口定位消息的用户，我能立即或加载后定位到目标，并继续
向上、向下浏览上下文。

**为什么优先**：定位不是一次性滚动动作，而是进入一个新的可分页阅读窗口。

**独立验收**：分别定位 Cache 命中和未命中的目标，然后从目标窗口向两个方向分页，并在期间注入新消息
和乱序定位响应，验证目标位置和用户意图稳定。

**验收场景**：

1. **给定** 目标消息已经在 Cache，**当** 用户定位它，**那么** 本地窗口选择器直接激活目标，不重新
   查询持久化数据。
2. **给定** 目标消息不在 Cache，**当** 用户定位它，**那么** 系统加载目标上下文、安装独立 Range，
   并按当前导航意图激活目标。
3. **给定** 用户已离开第一次定位并发起第二次定位，**当** 第一次响应迟到，**那么** 数据可以进入
   Cache，但第一次导航不得覆盖第二次定位结果。
4. **给定** 用户位于定位窗口，**当** 新消息到达频道底部，**那么** 当前定位 anchor 不动，并提示
   存在更新消息。
5. **给定** 用户从定位窗口选择返回最新，**当** 最新窗口仍驻留时本地切换；已淘汰时从当前持久化
   事实重新建立最新窗口。
6. **给定** 目标未缓存且其上下文双侧数据充足，**当** 以 `pageSize=N` 定位，**那么** 返回总数包含
   目标，目标上方为 `floor(N/2)` 条、下方为 `N-1-floor(N/2)` 条，并以目标作为视觉居中 anchor。
7. **给定** 目标一侧的数据不足，**当** 建立定位窗口，**那么** 未使用额度自动补给另一侧，总数不
   超过 `pageSize`，不足方向明确标记耗尽。

---

### 用户故事 5：多频道切换时 Cache 有界且状态正确（优先级：P2）

作为加入大量频道的用户，我可以快速切换近期频道，同时长期使用不会让完整消息 Cache 无限增长；
冷频道再次进入时仍能从持久化事实恢复。

**为什么优先**：Cache 是交互优化而不是第二事实源。无限常驻每个频道的最新消息会使内存随频道数
持续增长。

**独立验收**：将 Cache 预算设置为可确定触发淘汰的小值，在多个频道间切换、发送和接收消息，验证
热窗口复用、冷窗口重建、Pending 对账和 Cache 上限。

**验收场景**：

1. **给定** 每个频道都有最新窗口引用，**当** 冷频道实体 Chunk 被淘汰，**那么** 系统保留该频道的
   最新持久化水位并将窗口标记为 cold。
2. **给定** 冷频道收到非 Pending 新消息，**当** 持久化完成，**那么** 系统更新频道水位和新鲜度，
   不恢复完整消息 Chunk。
3. **给定** 用户在 B 群发送消息后切换到 A 群，且 B 群 Chunk 被淘汰，**当** B 群服务端终态到达，
   **那么** Pending 实体仍通过稳定临时身份完成对账。
4. **给定** 一个近期频道窗口仍驻留且对应持久化 revision 未变化，**当** 用户切回，**那么** 系统
   可以复用该窗口。
5. **给定** 当前窗口、视觉 anchor、Pending 实体或在途请求仍需要某个 Chunk，**当** Cache 执行
   淘汰，**那么** 该 Chunk 在需要期间不会被移除，并在条件结束后允许释放。

---

### 用户故事 6：断档时安全恢复（优先级：P1）

作为遇到重连、漏帧、乱序或非法 Delta 的用户，我继续看到最后一份完整正确的 Timeline，系统在后台
恢复权威窗口，不展示部分提交或自行猜测的数据。

**为什么优先**：错误恢复决定协议能否在真实网络和跨端运行中保持可信。

**独立验收**：注入 revision 断档、错误 epoch、无效 Range 引用和多 Surface 事务失败，验证原画面
保持、整帧拒绝、单次恢复和 Snapshot 后继续增量。

**验收场景**：

1. **给定** 当前 revision 为 20，**当** 直接收到以 21 之后状态为基础的 Delta，**那么** 系统不应用
   该 Delta，保持最后正确画面并请求恢复。
2. **给定** Delta 引用不存在或边界不匹配的 Range，**当** 校验失败，**那么** 整个 Frame 不提交。
3. **给定** 多次错误同时触发恢复，**当** Snapshot 查询开始，**那么** 同一作用域只运行一次恢复。
4. **给定** 当前视觉 anchor 仍存在，**当** Snapshot 恢复完成，**那么** 系统优先恢复 anchor 周围
   窗口；anchor 不存在时才回到最新窗口。
5. **给定** Frame 同时更新 Timeline 和其他 Surface，**当** 任一 mutation 非法，**那么** 用户只能
   看到事务前或事务后的完整状态，不能看到混合状态。

---

### 用户故事 7：切断 Timeline V2 而不误伤其他 Surface（优先级：P1）

作为跨端发布负责人，我可以将 Timeline Producer 和 Consumer 协调切换到 V3，并在稳定后删除 Timeline
V2，同时让没有变化的 Dialog 等 Surface 继续使用原合同。

**为什么优先**：Timeline V3 不能通过长期双轨或静默回退维持；也不能为了一个 Surface 的变化扩大
其他 Surface 的迁移风险。

**独立验收**：分别重放 V2 Timeline、V2 Dialog-only、V3 Timeline-only 和 V3 混合事务，验证版本
接受矩阵，再执行删除检查确保不存在 Timeline V2 运行入口。

**验收场景**：

1. **给定** V2 Frame 只包含非 Timeline Surface，**当** 已支持客户端接收，**那么** 原合同继续工作。
2. **给定** V2 Frame 包含 Timeline，**当** PC 或 Mobile 接收，**那么** 它 fail closed，且不得降级到
   原始事件或全量替换。
3. **给定** V3 Frame 同时包含 Timeline V3 和 Dialog V2 mutation，**当** 应用事务，**那么** 两者按
   各自合同校验，并保持整帧原子性。
4. **给定** V3 已通过真实链和回归验收，**当** 执行收尾，**那么** 各仓 Timeline V2 Producer、
   Consumer、Fixture 和兼容分支被物理删除，其他 Surface V2 不受影响。

---

### 用户故事 8：跨端测试资产与 Producer 合同同步升级（优先级：P1）

作为 Timeline V3 发布负责人，我需要 Helix Producer 测试、Helix 产出的 canonical JSON、PC Mock
和 Consumer 测试、Mobile 同步脚本与 Consumer 测试作为同一个迁移单元升级，确保测试真正验证
V3 Delta，而不是继续用 V2 `replaceView` 得到相似的最终页面。

**为什么优先**：如果只改业务运行时代码，旧 Fixture、Mock 和测试仍可能接受全量替换，从而让合同
回归在测试中保持绿色。Producer 产物和所有 Consumer 必须共同锁定操作级语义。

**独立验收**：选择发送、撤回、Reaction、加急、上拉和定位场景，让 Helix 从实际生产路径生成 V3
Fixture，再由 PC 与 Mobile 原样同步和消费，验证三端 operation、实体身份、Range 和最终状态一致。

**验收场景**：

1. **给定** Helix 为发送消息生成 canonical Fixture，**当** sending、sent 和 failed 阶段被测试，
   **那么** sending 使用 Upsert 与插入建立稳定实体，sent/failed 只 Upsert 同一实体，任何阶段都
   不含 Timeline `replaceView`。
2. **给定** 消息被撤回、添加 Reaction 或更新为加急状态，**当** Helix 生成对应 Fixture，**那么**
   每个变化只 Upsert 目标消息，不删除、不移动、不替换完整 Timeline。
3. **给定** Helix 已生成 canonical V3 JSON 和 provenance，**当** PC Mock 或 Mobile 测试同步，
   **那么** 它们保持 byte-exact，不改写业务字段、操作种类、EntityKey 或 Range 顺序。
4. **给定** PC 与 Mobile 消费同一 V3 Fixture，**当** 各自应用 Frame，**那么** 它们产生等价的
   归一化实体、Range、边界和窗口状态，并分别完成本端渲染断言。
5. **给定** 一个正向 Timeline Fixture 仍包含 `timelinePatch + replaceView`，**当** 执行 V3
   hard-cut Gate，**那么** Gate 必须失败，不能因最终页面正确而放行。
6. **给定** 一个 V2 Fixture 只包含未变化的非 Timeline Surface，**当** 执行迁移检查，**那么** 它
   继续作为合法资产保留，不被 Timeline hard-cut 误删。
7. **给定** 既有 G-01/G-02/G-03a/G-04/G-04a/G-05/G-06/G-07/G-10/G-10a/G-11/G-14/G-14a
   Timeline Gate，**当** 开始新增正式 Gate，**那么** 这些 Gate 的 Helix Producer、canonical
   JSON 与 provenance 必须已先完成 V3 operation 级迁移。
8. **给定** 一个定位或中间 Range，**当** G-10b 以 `pageSize=20` 或 60 执行下拉分页，**那么**
   Helix 只发布尾部 Upsert、Insert 与必要 Bounds 更新，不自动激活最新窗口，PC 与 Mobile 均无
   重复、遗漏或 anchor 抢占。
9. **给定** RenderWindow 已达到当前 `pageSize`，**当** G-10c 收到一条最新消息，**那么** Helix
   只追加稳定实体与 key；Consumer 在本地淘汰头部可见项并保持窗口上限，不把淘汰表达为业务
   delete、move、trim 或新的 Timeline revision。
10. **给定** 用户已经离开底部，**当** G-10d 收到新消息，**那么** Helix 更新实体、Range 与水位
    但不激活最新窗口，Consumer 保持视觉 anchor、增加新消息提示，并且只在用户点击提示后返回最新。
11. **给定** 目标消息未缓存，**当** G-11a 定位并继续双向分页，**那么** Helix 先安装含目标的
    居中 Segment 并按当前 token 激活，再分别向 start/end 扩展；迟到 token 只能入 Cache，不能
    抢占活动窗口。
12. **给定** 四组正式 Gate 的 Helix JSON 与 provenance 尚未 Green，**当** PC 或 Mobile 尝试
    新增对应 Consumer Fixture，**那么** 同步必须停止，Consumer 不得自行定义业务 JSON。

## Loop Contract *(mandatory)*

> 这是本功能的外部行为合同。实现机制归 `plan.md`，当前源码事实和固定输入归 `discovery.md`。

### Trigger

以下任一事件启动 Timeline V3 路径：用户首次进入或重新进入频道、发送或接收消息、消息权威状态
变化、用户触发双向分页或定位、缓存窗口变冷后恢复、连续版本出现断档，或发布负责人启动
Timeline V2 到 V3 的协调切换。

### Goal

用户在 PC 与 Mobile 获得同一份 Helix 权威 Timeline：首屏和恢复使用完整 Snapshot，普通变化使用
有界原子 Delta；消息身份、顺序、分页边界、导航意图和恢复行为保持确定，平台只机械应用合同，
不从原始事件重建业务事实。

### Completion

完成必须同时证明：受影响场景由 Helix 权威路径产生符合 V3 操作级语义的 canonical Frame；
PC 与 Mobile 对同一固定输入得到等价的实体、Range、边界、水位和活动窗口；可见终态能关联到已提交
的持久化事实；断档与非法事务均整帧拒绝并恢复；所有受影响 Gate 的 Producer、Fixture、Consumer
与 hard-cut 检查均通过。Mock、单端最终画面、传输成功或静态扫描不能单独作为完成证据。

### Boundaries

- 本功能只升级 `message.timeline`；Dialog、UnreadSummary、ThreadDetail、ChannelSummary、
  Composer、MemberRoster、TopicList 和 ParticipantReceipts 的合同不随本功能升级。
- Web 仍属于 Helix 产品目标集合，本 Feature 要求共享合同保持 Web 可消费，但不把产品级 Web
  Timeline Consumer 与真实浏览器验收纳入本次交付；因此本 Feature 单独完成不构成
  `ThreePlatformGreen`。
- 平台保留像素级滚动、焦点、动画和虚拟列表实现；这些状态不进入跨端业务合同。
- 本阶段只做有界、确定性的缓存淘汰验证，不承担百万消息、消息风暴或长期内存压力测试。
- 不引入通用 JSON Patch、数组下标增量、长期 V2/V3 双发、能力协商或运行时 V2 fallback。
- 自动恢复在同一 Timeline 作用域只允许一个在途 Snapshot；连续无法恢复或发现权威合同冲突时停止
  自动推进并进入人工决策，不猜测数据。

### Human Gate

以下变化必须获得显式批准：改变稳定实体身份或排序键、扩缩 V3 操作集合、允许运行时 V2 fallback、
改变跨仓协调切换顺序、物理删除 Timeline V2 资产、或把尚未持久化的数据提升为本地权威事实。
当前 Go newer 查询只有单列时间边界。用户已于 2026-07-28 显式批准把 cses-im-server 纳入变更
范围，新增 `(createAt, temporaryId)` 复合 cursor/order 和配套 PostgreSQL 索引 SQL；生产 DDL 仍
必须由环境所有者手工执行。G-10b/G-11a 只有在该生产链通过后才能从 projector 合同证据晋级
canonical Producer Green。其余符合本规格且保持现有边界的实现选择可在 Plan 阶段决定。

### Edge Cases

- 同一 `createAt` 下存在多条消息时，复合身份必须提供完整、稳定、无重复的分页顺序。
- Snapshot 或 Delta 中出现空、重复、冲突或可变的 EntityKey 时必须整帧拒绝。
- `hasOlder/hasNewer` 与方向 cursor 不一致时不得部分应用。
- 新消息在客户端持久化层已提交但目标频道不驻留时，只更新轻量频道事实，不恢复完整 UI Cache。
- Pending 消息在 Cache 淘汰、重连、账户切换或 Snapshot 恢复期间必须保持可对账。
- 用户快速交替上拉、下拉、定位和返回最新时，只有当前显式导航意图能改变活动窗口。
- Range 合并不能跨越未经证明的数据缺口。
- 撤回、发送失败和业务更新不能通过删除或移动稳定 EntityKey 表达。
- Cache 淘汰不得推进 Timeline revision，也不得删除持久化消息。
- 尚未同步到客户端持久化层的远端数据，不得被描述为本地已经存在的更多消息。
- V3 切换失败只能通过发布版本回滚，不能在运行时静默恢复 Timeline V2。
- Fixture 只修改文件名或顶层版本、内部仍为 `replaceView` 时必须判定为未迁移。
- PC 或 Mobile 对 Helix Fixture 补写业务字段、重排 keys 或改变 operation 时必须判定 provenance
  失败。
- Hash 工具若忽略 operation、EntityKey、Range、Bounds 或水位变化，必须判定为不满足 V3
  semantic parity。
- 一个 Gate 的 Producer、Fixture、Mock、Consumer 或 hard-cut 任一层仍停留在 Timeline V2 时，
  该 Gate 不得声明完成。

### Formal Gate Matrix

| Gate | 测试类 | Helix operation 合同 | Consumer 行为合同 |
|---|---|---|---|
| G-10b | `LoadNewerGate` | `upsertMessage* + insertKeys(end) + setRangeBounds?`，禁止隐式 `activateWindow` | 下拉分页无重复/遗漏，原 anchor 不被抢占 |
| G-10c | `FullWindowAppendEvictionGate` | `upsertMessage + insertKeys(end)` 与必要 Bounds，禁止 delete/move/trim/replace/activate | 满窗口本地头部淘汰，窗口保持 `pageSize`，业务 revision 不因淘汰变化 |
| G-10d | `AnchoredNewMessageGate` | 离底新消息更新实体、Range 与水位，禁止隐式 `activateWindow` | anchor 偏移不变，提示计数增加，点击后才返回最新 |
| G-11a | `UncachedLocateBidirectionalGate` | `upsertMessage* + installSegment + activateWindow(token)`，随后 older/newer 只扩展 Range | 目标居中、双向分页、迟到 token 不抢活动窗口 |

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**：系统必须由 Helix 独占 Timeline 消息身份、顺序、可渲染实体、Range、revision 和恢复
  语义；客户端不得从原始事件重建这些业务事实。
- **FR-002**：系统必须将 Timeline V3 定义为版本化合同，并使任何包含 Timeline 的 Frame 使用 V3。
- **FR-003**：系统必须允许不含 Timeline 的 V2 Frame 和未变化 Surface 合同继续工作。
- **FR-004**：V2 Frame 一旦包含 Timeline，所有已迁移 Consumer 必须 fail closed。
- **FR-005**：V3 混合事务必须允许 Timeline V3 与其他未变化 Surface 合同共存，并保持整帧原子性。
- **FR-006**：普通登录、首次挂载和冷频道重进必须以完整 Snapshot 建立最新消息窗口。
- **FR-007**：Snapshot 必须携带完整可渲染实体、有序 Range、活动窗口、双向边界、持久化水位和
  连续版本状态。
- **FR-008**：普通消息变化必须通过有界 Delta 表达，不得发布 Timeline 全量 `replaceView`。
- **FR-009**：Delta 只能使用 PRD 定义的 `upsertMessage`、`installSegment`、`insertKeys`、
  `mergeRanges`、`setRangeBounds` 和 `activateWindow`。
- **FR-010**：消息必须使用不可变的 `createAt + temporaryId` 复合身份；服务端 ID 不得改变实体
  身份或排序位置。
- **FR-011**：发送消息从 sending 到 sent 或 failed 必须更新同一实体，并保持同一可见行身份。
- **FR-012**：撤回、回复、Reaction、回执和发送状态变化必须通过完整单实体 Upsert 表达。
- **FR-013**：系统不得提供 Timeline 业务删除或消息移动操作；撤回和失败保留原实体。
- **FR-014**：消息实体更新与 Range 排序必须分离，客户端不得按消息字段重新排序。
- **FR-015**：最新、上拉和下拉必须使用稳定复合游标分页，并以额外一条记录判断对应方向是否还有
  数据。
- **FR-016**：`hasOlder`、`hasNewer`、方向 cursor 和频道最新水位必须来自已提交的客户端持久化
  事实，客户端展示层不得推断。
- **FR-017**：位于底部时，新消息必须自动跟随；离开底部后，新消息必须保持视觉 anchor 并只提示
  存在更新。
- **FR-018**：普通上拉、下拉、消息更新、Cache 预热和 Range 合并不得激活或移动活动窗口。
- **FR-019**：显式定位、返回最新、跳未读或恢复需要服务端加载时，窗口激活必须绑定当前导航意图。
- **FR-020**：过期导航响应可以补充 Cache，但不得覆盖用户之后发起的导航。
- **FR-021**：定位命中 Cache 时必须允许本地激活；未命中时必须安装可双向分页的独立上下文 Range。
- **FR-022**：系统必须维护标准化、按频道隔离的实体和 Range Cache，RenderWindow 不得复制第二份
  业务实体。
- **FR-023**：Cache 必须同时受实体数量和估算字节预算约束，并以 Chunk/Page 为淘汰单位。
- **FR-024**：当前窗口、视觉 anchor、Pending 对账和在途分页或定位涉及的数据必须在需要期间保持
  Pin，并有确定的释放条件。
- **FR-025**：所有频道必须保留轻量最新窗口引用；只有热频道可以保留完整最新消息 Chunk。
- **FR-026**：冷频道收到非 Pending 更新时必须更新持久化事实和窗口新鲜度，不得强制恢复完整 Cache。
- **FR-027**：冷或陈旧频道重新进入时必须重新读取最新窗口；只有 revision 一致的驻留窗口可以复用。
- **FR-028**：每个 Timeline Delta 必须匹配当前 epoch 和 base revision，并作为单一事务暂存、校验
  和提交。
- **FR-029**：任一 mutation、Range 边界或版本校验失败时，整个 Frame 必须不提交。
- **FR-030**：断档恢复期间系统必须保留最后正确画面、合并重复恢复请求、保护 Pending 对账，并优先
  恢复当前 anchor。
- **FR-031**：恢复 Snapshot 成功后，系统必须丢弃其 revision 之前的迟到 Delta，再继续增量消费。
- **FR-032**：Canonical V3 Fixture 必须覆盖 Snapshot、全部 Delta 操作、重复、断档、乱序、定位、
  淘汰、混合事务和 V2 Timeline 拒绝。
- **FR-033**：PC 与 Mobile 必须对同一 canonical Fixture 产生等价的归一化 Timeline 状态。
- **FR-034**：V3 切换前必须全面关闭 Timeline V2 的运行时生成和消费入口，不允许长期双发、能力协商
  或运行时 V2 降级。
- **FR-035**：V3 真实链和回归验收平稳后，必须物理删除各仓 Timeline V2 Producer、Consumer、
  Fixture 和兼容分支，同时保留其他 Surface 所需 V2 合同。
- **FR-036**：所有受 Timeline V3 影响的 Helix Producer 测试、canonical Fixture、PC Consumer
  测试和 Mobile Consumer 测试必须纳入功能范围，不能只修改业务运行时代码。
- **FR-037**：正向 canonical Timeline Fixture 必须由 Helix 实际 Producer/Projector 路径生成，
  Consumer 仓不得手工编写或修改业务 mutation。
- **FR-038**：每个 canonical Fixture 必须携带可验证的 Producer revision、文件身份、raw hash、
  semantic hash、Schema/Surface 版本、Surface inventory 和允许参数化字段。
- **FR-039**：PC 与 Mobile 必须从同一个固定 Helix revision byte-exact 同步 canonical Fixture，
  只允许参数化 provenance 明确声明的运行时字段。
- **FR-040**：Semantic parity 必须覆盖 operation 种类与顺序、EntityKey、Range 位置、Bounds、
  水位和 revision，不能只覆盖最终 Message ViewModel。
- **FR-041**：发送场景测试必须分别断言 sending 的 Upsert/插入和 sent/failed 的同实体 Upsert，
  并证明不存在第二实体、第二消息行或 Timeline `replaceView`。
- **FR-042**：撤回、Reaction、加急、模板、回执及其他消息行更新测试必须断言只 Upsert 目标实体，
  不得以最终字段值相同替代操作级验证。
- **FR-043**：上拉、下拉、定位、返回最新和 Range 合并测试必须分别断言其 Range 操作、cursor、
  边界和窗口激活条件。
- **FR-044**：PC 测试必须覆盖 V3 合同解码、原子事务、Cache/Range、导航、Snapshot 恢复、稳定
  渲染行身份、Mock Fixture parity 和真实 Frame 观测。
- **FR-045**：Mobile 测试必须覆盖 V3 解码、Delta 应用、Cache/Range、窗口、恢复、V2 Timeline
  拒绝和与 PC 等价的归一化结果。
- **FR-046**：V3 hard-cut Gate 必须拒绝任何正向 Timeline V2 Fixture、生成逻辑或 Consumer 期待，
  同时允许不含 Timeline 的 V2 Surface 资产继续存在。
- **FR-047**：每个受影响 Gate 只有在 Helix Producer、canonical JSON/provenance、PC 同步与测试、
  Mobile 同步与测试和 hard-cut 检查全部通过后才能声明迁移完成。
- **FR-048**：首屏、上拉、下拉和定位必须接受同一个有界 `pageSize` 合同；缺失时默认 20，合法范围
  为 `1..=60`，非法值必须 fail closed。
- **FR-049**：前端只能声明窗口数量，不能提交排序、Range、cursor、EntityKey 重写或服务端边界。
- **FR-050**：首屏、上拉和下拉必须以持久化 lookahead 或服务端权威剩余计数判断是否还有数据；
  lookahead 记录不得进入渲染窗口，固定小批次服务端必须在有界轮数内累计到 `pageSize` 或耗尽。
- **FR-051**：定位窗口的总实体数必须包含目标；双侧充足时目标上方数量为
  `floor(pageSize/2)`，下方数量为 `pageSize-1-floor(pageSize/2)`。
- **FR-052**：定位一侧不足时必须把未使用额度补给另一侧，同时保持目标为视觉 anchor，并准确标记
  已耗尽方向。
- **FR-053**：新增正式 Gate 前，既有 13 个 Timeline 业务 Gate 的 Helix Producer、canonical
  JSON 与 provenance 必须先完成 V3 operation 级迁移。
- **FR-054**：G-10b 必须覆盖下拉分页、更新方向耗尽和耗尽后迟到响应；正向 Delta 只能尾部
  Upsert/Insert 和必要 Bounds 更新，不得隐式激活最新窗口。
- **FR-055**：G-10c 必须证明满窗口 append 只产生尾部 Upsert/Insert；头部可见项淘汰是 Consumer
  本地有界窗口行为，不得编码为 Helix delete、move、trim 或额外 revision。
- **FR-056**：G-10d 必须证明离底新消息不激活最新窗口，Consumer 保持 anchor、累计新消息提示，
  并仅在显式点击提示后返回最新与清除提示。
- **FR-057**：G-11a 必须覆盖未缓存定位的 Segment 安装、当前 navigation token 激活、目标居中、
  定位后 older/newer 双向分页和 stale token 不抢占。
- **FR-058**：四组正式 Gate 必须各自拥有独立测试类、canonical 目录、`.frame-v3.json` inventory
  与 provenance，不得复用旧 Gate 名称隐藏发布分母变化。
- **FR-059**：正式执行顺序必须是既有 Helix Timeline Gate、四组新 Helix Gate、PC Fixture/WDIO、
  Mobile Fixture/GTest、三仓 parity/hard-cut；前序未 Green 时后序 Consumer 不得手写替代资产。
- **FR-060**：PC WDIO 必须对四组正式 Gate 执行 operation、Range、Bounds、window、anchor、提示与
  点击跳转断言，不能只比较最终 DOM 文本。
- **FR-061**：Mobile GTest 必须对四组正式 Gate执行与 PC 等价的 operation、normalized state、
  window、anchor、提示与点击跳转断言。
- **FR-062**：canonical semantic hash 必须覆盖 operation 种类与顺序、EntityKey、Range 位置、
  Bounds、水位、revision 和活动窗口；省略任一字段的 hash 不具备 V3 parity 证明力。
- **FR-063**：发送终态、撤回、Reaction、加急、模板、回执及其他普通消息更新的正向 Delta 只能
  Upsert 目标稳定实体；除显式导航外不得附带 `activateWindow`，任何 Timeline 全量 Replace 均失败。

### Key Entities

- **Timeline EntityKey**：由不可变创建时间和客户端持久化主键组成的消息稳定身份与排序键。
- **Timeline Entity**：Helix 已裁决的完整可渲染消息；状态更新不会改变 EntityKey。
- **Timeline Range**：一个频道内按 EntityKey 严格有序、内部连续的消息 key 序列。
- **Range Bounds**：一个 Range 在更早和更新方向的 cursor 与是否仍有持久化数据。
- **Timeline Snapshot**：在指定 epoch、revision 和持久化水位下原子建立实体、Range 与活动窗口。
- **Timeline Delta**：从 base revision 到结果 revision 的原子、有界 Timeline 状态转换。
- **RenderWindow**：从一个 Range 选择出的有界可见窗口，包含本地 anchor 和滚动状态。
- **Channel Latest Window Reference**：每个频道常驻的轻量最新水位、已加载边界和 resident/cold/stale
  状态。
- **Pending Pin**：保证发送中实体在终态对账前不被 Cache 淘汰的有界引用。
- **Navigation Token**：标识当前显式定位、返回最新、跳未读或恢复意图，阻止迟到响应抢占窗口。
- **Window Size Policy**：前端可选择的有界窗口数量；默认 20、合法范围 `1..=60`，同一值约束首屏、
  上拉、下拉、定位和 Consumer RenderWindow。
- **Timeline Epoch/Revision**：区分缓存世代和连续 Delta 顺序的协议状态。
- **Persistence Revision**：表示当前窗口对应客户端持久化事实的新鲜度水位。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**：除首次加载、冷缓存重建和明确恢复外，100% 普通消息变化均不产生 Timeline 全量
  `replaceView`。
- **SC-002**：发送生命周期的全部合同与 UI 测试中，每次发送始终只有一个稳定实体和一个消息行。
- **SC-003**：首屏、上拉、下拉、定位、返回最新和 Range 合并的全部确定性测试中不存在重复或遗漏
  消息。
- **SC-004**：离开底部、定位窗口和乱序响应的全部测试中，非当前导航意图造成的滚动跳转次数为 0。
- **SC-005**：全部方向边界和最新水位断言都能关联到已提交的客户端持久化查询结果，展示层推断次数
  为 0。
- **SC-006**：Cache 小预算测试中，实体数量和估算字节均不超过配置上限；所有受保护实体在释放条件
  前保持可用，释放后可以被淘汰。
- **SC-007**：可见、Cache 内不可见和已淘汰消息的更新测试均得到预期结果，且不恢复无关冷 Chunk。
- **SC-008**：100% epoch、revision、Range、cursor 和混合事务错误 Fixture 均整帧 fail closed，
  不存在部分提交。
- **SC-009**：恢复测试中用户始终看到恢复前完整画面或恢复后完整画面，空白或中间状态次数为 0。
- **SC-010**：PC 与 Mobile 对 100% canonical V3 Fixture 产生等价的归一化实体、Range、边界和窗口
  状态。
- **SC-011**：V3 切换 Gate 能机器证明 Timeline V2 运行时 Producer 和 Consumer 均为 0 个，同时
  V2 非 Timeline Surface 回归保持通过。
- **SC-012**：V3 稳定验收后，代码和 Fixture 扫描中 Timeline V2 `timelinePatch + replaceView`
  生成与消费实现为 0 个。
- **SC-013**：发送、撤回、Reaction、加急、上拉、下拉和定位的 100% 正向 canonical Fixture 均
  使用规定的 Timeline V3 operation，Timeline `replaceView` 出现次数为 0。
- **SC-014**：每个受影响 Gate 的 Helix、PC 和 Mobile Fixture raw hash、semantic hash、Producer
  provenance 与 Surface inventory 全部一致。
- **SC-015**：PC 与 Mobile 对 100% canonical Timeline V3 Fixture 不仅解码成功，而且输出等价的
  归一化实体、Range、Bounds、水位和活动窗口。
- **SC-016**：所有发送终态、撤回、Reaction 和加急测试中，目标 EntityKey 保持不变，额外实体或
  渲染行数量为 0。
- **SC-017**：Timeline V3 hard-cut 检查对任意正向 `timelinePatch + replaceView` 样本均失败，
  同时所有未变化的非 Timeline V2 Fixture 回归通过。
- **SC-018**：首屏、上拉、下拉和定位在 `pageSize=20` 与 `pageSize=60` 的全部确定性测试中均满足
  数量上限、无重复、无遗漏和边界一致；`0/61` 的拒绝率为 100%。
- **SC-019**：定位双侧充足的全部测试中，目标索引严格等于 `floor(pageSize/2)`；单侧不足时总数不
  超过 `pageSize`，可用额度浪费为 0。
- **SC-020**：既有 13 个 Timeline Gate 的全部正向 Helix Fixture 中 Timeline Replace 为 0，
  普通终态或业务更新的额外 `activateWindow` 为 0。
- **SC-021**：G-10b/G-10c/G-10d/G-11a 在 `pageSize=20/60` 的 Helix 测试、canonical inventory
  和 provenance 全部通过，且 `0/61` 请求 100% fail closed。
- **SC-022**：PC 对四组正式 Gate 的 operation 级合同、标准化状态与 DOM/anchor/提示/点击行为
  全部通过，且 Fixture 与 Helix byte-exact。
- **SC-023**：Mobile 对四组正式 Gate 的 operation 级合同、标准化状态与 window/anchor/提示/点击
  行为全部通过，且 Fixture 与 PC/Helix byte-exact。
- **SC-024**：Strict Matrix 的版本化分母显式新增 G-10b/G-10c/G-10d/G-11a；三仓报告对同一 Matrix
  epoch、Producer revision、raw hash 与 semantic hash 一致，任一漂移均 fail closed。

## Assumptions

- Helix 继续作为跨端 Timeline 合同、排序和可渲染消息语义的唯一权威方。
- 客户端持久化层已经为每条消息提供非空且稳定的 `temporaryId`。
- `createAt` 与 `temporaryId` 在消息生命周期内不可改变；任何无法满足该不变量的数据必须在迁移前
  修复或被严格拒绝。
- PC 和 Mobile 可以在同一协调发布窗口完成 Timeline V3 Consumer 切换。
- 非 Timeline Surface 可以继续独立使用 V2 Frame 或在 V3 混合事务中复用其现有合同。
- Helix 生成的 canonical JSON 和 provenance 是 PC、Mobile Fixture 的唯一业务真源。
- 现有 Gate 测试分类、Mock/integration/real 证据等级继续保留；Fixture 或 Mock 通过不能替代真实
  持久化、Frame 和渲染证据。
- Web Consumer 的产品化迁移由后续独立 Feature 承担；本 Feature 的共享合同不得引入阻止 Web
  消费的平台依赖。
- UI 可以维护像素级视觉 anchor，但该值不进入跨端业务合同。
- 本功能只要求确定性功能淘汰测试；容量压测和长期运行压测由后续性能工作单独定义。
- V3 失败时使用版本发布回滚，不提供运行时 Timeline V2 fallback。
