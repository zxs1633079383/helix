# MessageV3 在线、离线与缺口补齐统一事件内核规范

## Problem Statement

当前 MessageV3 对同一服务端 `channel_event` 事实存在两套本地应用语义：在线 WebSocket type=1 会在原子事务中更新消息、共享频道摘要、当前用户的 `channel_member` 未读绝对态来源和事件游标；离线 Sync type=1 则更新消息、共享 `channel` 未读字段和事件游标。两条路径提交后读取的对象也不同，且 Sync 的 Dialog 投影还受时间线是否附着、是否处于 PongGap 批次等 UI/恢复状态门控。

这会产生可稳定解释的跨层偏差：用户 678 从群聊 A 切换到群聊 B 后，用户 444 在 A 发送第一条消息；若 678 此时检测到 gap 并通过 Sync 补齐，第一条消息可以进入本地消息表并推进游标，却没有以 `(channel_id, auth_user_id)` 更新并读回当前用户未读权威，也可能因为 A 的时间线未附着而不发布 Dialog 绝对态。第二条消息随后走在线链路，从仍为 0 的 `channel_member.unread_count` 加到 1，于是界面表现为“第一条不更新，第二条才显示 1”，而不是预期的 2。

这不是 Go 漏推一个兼容事件，也不是 Angular 少做一次 `+1`。根因是 Helix 对同一事件事实按入口选择了不同 reducer、不同未读权威和不同投影释放条件。继续在 Go 增加 `update_channel`、在 Angular 猜未读，或保留在线/Sync 双写，只会扩大协议和状态空间，无法证明重放、乱序与重启恢复的一致性。

本次工作同时承担性能收敛：不能把“在线和 Sync 对齐”实现成每个补齐 Event 各自写库、读回和发事件。批量恢复必须在不改变逐事件业务语义的前提下，聚合为每个频道一次原子提交和一次 viewer 复合键读回，避免恢复 N 条消息时产生 O(N) 次数据库往返和 O(N) 条重复 Dialog 投影。

## Solution

在 `helix-im` 建立唯一的 viewer-scoped 事件应用内核。这里的 Core 指无 UI、无平台、无数据库实现依赖的 IM 领域内核，不是让通用 `helix-core` 认识消息、群聊或未读字段。在线 WebSocket、Routine Sync、断线恢复、PongGap、Hydration 和缓冲事件连续推进都只负责把权威输入规范化为同一种事件信封，然后调用同一事件 mutation compiler；入口不再拥有第二套业务 reducer。

统一内核把每个权威 Event 编译为四类结果：消息事实写集、共享频道摘要写集、当前 viewer 的成员态写集、提交成功后待发布的领域投影。它同时返回受影响频道和是否需要 Dialog 绝对态读回。编译结果不执行 IO、不直接 Emit，也不依赖当前打开哪个群聊。

type=1 的统一语义为：可见消息 upsert；共享 `channel` 只更新频道级 lastPost 排序摘要，不保存当前用户未读权威；以 `(channel_id, auth_user_id)` 对 `channel_member` 做幂等创建和有序 `ScopedGuardedBump`；同一事务最后写 recovery ledger、coverage 与 cursor。发送者自己的消息更新 lastPost 但未读不增加。不可见消息和缺少 messages 快照的 phantom 不落消息、不增加未读、不发布消息或 Dialog 投影，但仍按既有事件完整性规则参与 ledger、coverage 和 cursor 判断。

type=2 的统一语义为：只按权威存在字段 patch 目标消息，不修改未读，不自行查询或猜测 lastPost，不提前 Emit。接龙声明继续由独立 Chain projection 处理，不能退化成普通消息编辑，也不能恢复旧公告兼容路径。

type=3 的统一语义为：只把目标消息标记为撤回并发布撤回投影，不修改未读，不因为撤回而本地 `-1`，不猜测前一条 lastPost。目标消息在本地不存在时保持幂等 no-op，但事件身份、ledger、coverage 和 cursor 仍按合法权威事实处理。

type=6 不是本次未读修复的权威来源，但必须作为混批顺序的一部分继续受统一事务约束：type=1 先建立消息，type=2/type=3 再应用内容终态，type=6 最后单调覆盖 readBits。type=6 不因缺少本地消息而创建消息，也不直接清空 Dialog 未读；当前 viewer 的已读绝对态只能来自既有明确的成员态权威合同。

在线单事件与 Sync 批处理复用同一编译器，但采用不同编排器：在线编排器提交一个 Event 的写集；Sync 编排器按服务端业务顺序编译 N 个 Event，合并写集，追加逐事件 ledger、批次 coverage 和一个最终 cursor，再以单个 `PersistAtomic` 提交。Sync 聚合器按受影响频道去重 Dialog dirty 标记，提交成功后每个频道只按 `(channel_id, auth_user_id)` 读回一次 `channel_member`，发布一条最终绝对 `im:channel:update`。因此业务计算为 O(N)，临时空间为 O(N + C)，数据库往返为每批一次原子写加每个受影响频道一次 O(1) 读回，其中 C 为批次涉及频道数；当前接口通常一批一个频道，故为固定两次往返。

`PersistAtomic` 是唯一发布屏障。Persist 失败、回报 corr 不匹配、批次不连续、event identity 冲突或 viewer scope 缺失时，不推进 cursor、不发布消息终态、不发布 Dialog 终态。匹配的 PersistOk 到达后才确认 cursor、结算 pending send、发逐消息事件并进行成员态读回。读回成功后发布唯一绝对 Dialog 投影；读回失败时保持已提交事实但不猜结果，并暴露可定位的结构化失败证据。

Dialog 投影与 Timeline 投影分离。是否打开目标群聊、是否附着 latest window 只决定是否刷新该时间线，不得决定左侧会话未读和 lastPost 是否发布。PongGap 可以把多个频道的 Dialog 投影延迟到批次终点合并释放，但不能吞掉最终投影；Hydration 可以抑制逐消息动画事件，但必须使 viewer 成员态和最终可恢复 Dialog snapshot 与在线结果一致。

Angular/Tauri 继续只消费 Helix 的绝对投影，不对未读执行 `+1/-1`，不根据消息到达顺序猜 lastPost，也不因缺失投影直接调用 Go 补偿。Go 继续提供现有 `channel_event.event_seq`、事件类型、messages 快照和既有成员态事实；本方案不新增 Go 字段、不增加 WebSocket 事件、不建立新旧协议双轨。

## User Stories

1. 作为停留在群聊 B 的用户，当群聊 A 收到第一条他人消息时，我希望 A 的未读立即变为 1，而不需要先打开 A。
2. 作为停留在群聊 B 的用户，当群聊 A 连续收到两条他人消息时，我希望 A 的未读最终为 2，不出现第二条到达后才显示 1。
3. 作为在线用户，当下一条 WebSocket 事件连续到达时，我希望消息、游标、未读和 Dialog lastPost 在同一次持久提交后一起可见。
4. 作为发生 gap 的在线用户，当缺失事件通过 Sync 补齐后，我希望它与原本在线连续到达产生完全相同的本地数据库终态。
5. 作为断线重连用户，当离线期间收到多条消息时，我希望恢复后的未读数等于全部可见且非本人消息的权威结果。
6. 作为重启恢复用户，当本地进程重新加载并执行 Hydration 时，我希望 Dialog snapshot 与未重启时一致，不依赖曾打开哪个群聊。
7. 作为消息发送者，当自己的消息 echo 在线或通过 Sync 回放时，我希望 lastPost 更新但自己的未读不增加。
8. 作为受限可见消息的非目标用户，当事件出现在同步范围但消息对我不可见时，我希望它不落成本地可见消息，也不增加未读。
9. 作为遇到 phantom Event 的用户，当 Sync 没有提供对应 messages 快照时，我希望系统不伪造消息、lastPost 或未读，但不会用本地猜测跨过不完整批次。
10. 作为收到重复在线推送的用户，我希望同一事件只应用一次，未读不重复增加。
11. 作为收到重复 Sync 批次的用户，我希望 ledger、guard 和 cursor 使回放成为幂等 no-op。
12. 作为遇到在线与 Sync 竞态的用户，我希望相同 Event 无论先从哪个入口到达都只形成一个数据库终态和一个 Dialog 绝对态。
13. 作为遇到乱序事件的用户，我希望后到高序号事件先进入缓冲，缺口补齐并提交后再连续推进，cursor 不跳过缺失业务序号。
14. 作为消息编辑的用户，我希望 type=2 在线与 Sync 都只修改服务端实际提供的字段，不改变未读。
15. 作为消息撤回相关用户，我希望 type=3 在线与 Sync 都形成相同撤回终态，不因本地猜测前一条消息而产生不同 lastPost 或未读。
16. 作为已读回执相关用户，我希望 type=6 在混批中晚于 type=1 应用，避免新消息 upsert 把新的 readBits 擦回旧值。
17. 作为文字接龙用户，我希望接龙声明和 Entry 的 canonical projection 仍走独立 Chain Domain，不被统一消息编辑逻辑误吞。
18. 作为文字接龙用户，当接龙事件通过在线、离线或 gap 补齐到达时，我希望既有 8 个文字接龙真实 Gate 的状态机、顺序和 UI 观察不回退。
19. 作为左侧会话列表用户，我希望未附着目标消息时间线时仍能收到其 Dialog 更新。
20. 作为当前打开目标群聊的用户，我希望 Timeline 可以按既有附着规则刷新，同时 Dialog 未读不会因为 Timeline 刷新而重复结算。
21. 作为 PongGap 恢复用户，我希望批次可以合并 UI 发布以减少抖动，但批次结束必须产生最终绝对 Dialog 状态。
22. 作为数据库暂时写失败的用户，我希望界面不显示未持久化的消息、未读或成功状态，重试后仍能从同一 cursor 恢复。
23. 作为成员态读回失败的用户，我希望系统不猜未读值；日志能够指出事件、频道、viewer、提交 corr 和读回失败阶段。
24. 作为客户端维护者，我希望 Angular/Tauri 只机械应用 Helix 绝对投影，不保留隐藏的未读 reducer 或 Go 直连补偿。
25. 作为后端维护者，我希望此次修复不要求 Go 增加兼容推送、调整 event type 或承担客户端局部状态补偿。
26. 作为性能维护者，我希望补齐 N 个 Event 时只进行一次批量原子写和每个受影响频道一次成员态读回，而不是 N 次写后读回。
27. 作为 Reviewer，我希望能够从一个 eventId/eventSeq、channelId、viewerId 和 correlation 追踪 accepted、persisted、projected 的每一跳。
28. 作为发布负责人，我希望 Phase1–4 的 80 个机器 Gate 和文字接龙 scoped 8 Gate 都有独立证据，任何未运行、跳过或失败都阻止声明整体 Green。

## Implementation Decisions

- 领域归属固定在 `helix-im`。通用 `helix-core` 只提供 Tick、Effect、StorageOp 和确定性运行机制，不承载 IM event type、未读或 Dialog 业务。
- 采用 Compiler + Orchestrator 模式：纯事件编译器决定业务 mutation；在线、Sync、PongGap 和 Hydration 编排器只负责输入规范化、批量聚合、相关性和发布时机。
- 事件编译器的输入必须包含规范化 Event、messages 权威快照和当前 viewer identity；缺少 viewer identity 时，任何 viewer-local mutation fail closed。
- 编译器输出显式区分 message ops、shared-channel ops、viewer-member ops、terminal events、dialog-dirty 和 skip reason，禁止调用方通过检查 SQL 文本反推业务语义。
- `channel_member(channel_id,user_id)` 是当前用户未读、mention、urgent 和 Dialog 成员态的唯一本地权威。共享 `channel` 不再写 viewer unread，也不作为 Dialog unread 读回来源。
- type=1 的 `ScopedGuardedBump` 使用当前 viewer 复合键和单调业务 guard；相同或更旧事件不能再次增加未读。guard 必须基于稳定业务顺序，不使用 WebSocket transport seq。
- type=1 对 sender view 只更新成员 lastPost，不增加未读；receiver view 对可见消息增加一次未读。该差异由同一编译器决定，不由入口决定。
- type=2、type=3、type=6 的 unread delta 恒为 0。没有明确服务端成员态合同前，不因编辑、撤回或 readBits 推导 Dialog 未读。
- Sync 的逐事件业务顺序沿用已冻结的 type1 → type2 → type3 → type6 桶序；ledger 仍逐 Event 写入，coverage 和 cursor 每批写一次。
- 所有事件事实、成员态更新、ledger、coverage 和 cursor 必须在同一 `PersistAtomic` 内；任何拆分提交都视为合同失败。
- 在线单事件也使用相同事件编译器，并保留一次 viewer 复合键读回和唯一 `im:channel:update`。
- Sync 批次按频道聚合 `dialog-dirty`，同批同频道只执行一次 viewer 复合键读回，只发布最终绝对值；逐消息 `im:post:*` 仍按权威业务顺序释放。
- Dialog 释放不依赖 Timeline attachment。Timeline refresh 仍可保持 attached-only，以免恢复时为未打开频道构建无用窗口。
- PongGap 的延迟发布器必须持有待发布频道集合，并在全部相关 PersistOk 后统一读回/释放；任一 PersistErr 使该批不能声明完整成功。
- Hydration 不要求逐条动画式 `im:post:received`，但其 durable member state、cursor 和最终 Dialog snapshot 必须与相同 Event 的在线回放一致。
- type=2 不保留 gap 时的 pre-Persist 提前 Emit、`PersistFire` 兼容路径。缺口事件必须等待补齐、原子提交和 PersistOk。
- type=3 不查询或猜测被撤回消息之前的 lastPost，不合并额外 `update_channel` 事件，也不改变 Go wire。
- Chain Domain 的 canonical projection 在统一事务中保持独立 mutation 和 terminal event；普通 type=2 编译器必须继续识别并跳过接龙声明。
- 不提供 legacy unread 双写、不保留 `channel.unread_count` 回退读取、不让 Angular 在缺失事件时补算。
- 结构化观测至少包含 hop、source、eventId、eventSeq、eventType、channelId、viewer scope、upstream corr、persist corr、operation summary、skip reason 和 projection outcome；永久指标不得把用户或频道 ID 作为标签。
- 实现阶段先以最高公开行为缝写失败测试，再修改生产代码；纯 helper 测试只能补充边界，不能替代 `ImModule::handle` 证据。
- 新增或修改的每个函数都要有紧邻精简注释，说明其职责、边界、不变量或非显然副作用。

## Testing Decisions

最高测试缝固定为公开 `ImModule::handle`：以在线 Inbound、Sync HTTP PortReply、PersistOk/PersistErr、成员态 readback PortReply 和 Timer/Pong 作为输入，只从真实生产调用链产生的 `Effect::PersistAtomic`、`Effect::Persist`、`Effect::Http` 与 `Effect::Emit` 断言结果。测试不得手写业务 Emit 或绕过 correlation map 调内部 reducer。

第一组 TDD 回归必须精确复现用户 678/444 场景：678 已附着 B、A 未附着；A 本地 cursor 落后；补齐 type=1 后断言同一 `PersistAtomic` 包含 message、shared lastPost、viewer `channel_member` scoped bump、ledger、coverage 和 cursor；PersistOk 前零终态 Emit；成员读回为 1 后发布唯一 Dialog 绝对态；第二条在线 type=1 后读回为 2。

事件对齐矩阵至少覆盖：

| Event | 在线连续 | 小 gap 缓冲后补齐 | 大 gap 立即 Sync | 断线 Routine Sync | PongGap | Hydration | 重复/迟到 |
|---|---|---|---|---|---|---|---|
| type=1 Post | 必测 | 必测 | 必测 | 必测 | 必测 | 必测最终 snapshot | 必测幂等 |
| type=2 Edit | 必测 | 必测 | 必测 | 必测 | 必测 | 必测最终消息态 | 必测幂等 |
| type=3 Revoke | 必测 | 必测 | 必测 | 必测 | 必测 | 必测最终撤回态 | 必测幂等 |
| type=6 Read | 必测顺序 | 必测顺序 | 必测顺序 | 必测混批 | 必测混批 | 必测最终 readBits | 必测单调 |

每个模式都必须覆盖目标频道 attached 与 unattached 两种 UI 状态；Dialog 结果应一致，只有 Timeline refresh 数量允许不同。type=1 另覆盖 sender、可见 receiver、不可见 receiver、phantom、陌生频道、mention/urgent 以及同毫秒或同 guard 边界。

存储集成缝使用真实 Native SQLite executor 回放 Helix Effects：执行原子写、回灌 PersistOk、执行复合键读回并回灌 rows；对账 `message`、`channel`、`channel_member`、recovery ledger、coverage 和 cursor。必须注入原子写失败、读回失败、重复 PersistOk、错误 corr 和进程重启，证明无部分发布和可恢复性。

性能验收使用固定批量 1、10、100、500 个 Event，统计 Helix Effects 而不是只测墙钟时间。每个单频道 Sync 批次必须恰好一个业务 `PersistAtomic`、至多一个 viewer 成员态读回和一条最终 Dialog 投影；不得随 Event 数线性增加数据库往返。编译时间与写集大小允许 O(N)，不得出现事件两两比较或频道全表扫描。

协议与边界测试必须证明 Go wire、event type 和现有 HTTP/WS endpoint 零变化；Angular/Tauri production boundary 中不存在未读 `+1/-1` reducer、Sync 专属兼容 reducer、Go 直连补偿或测试 Fixture 可达路径。

整体无回退验收不复制第二份 Gate 清单，运行时直接读取客户端 Phase1–4 manifest 的当前机器分母。当前基线是 Phase1 33、Phase2 20、Phase3 15、Phase4 12，共 80 Gate；实施完成后 required Gate 必须 fail=0、skip=0，且报告逐 Phase 展示，不得只给总 PASS。

文字接龙作为独立 Phase5 回归。完整 manifest 为 12 Gate，但本次已有实现范围只要求并准确报告 Common G01–G06 与 Text G01–G02 共 8 Gate；Category 三 Gate与 Acceptance 一 Gate 不得混入 scoped PASS。8 个 scoped Gate 必须覆盖在线、离线恢复或 gap 参与时仍由独立 Chain projection 产生，不能被 type=2 普通 patch、旧公告兼容或批处理聚合吞掉。

本次编码完成后先执行 focused code review，只审查统一事件内核、旧分支删除、相关测试和本次 diff。focused review 完成后必须暂停，向用户明确询问是否继续执行 Phase1–4 的在线、离线与 gap 整体 Review；没有用户确认不得自动启动这轮大范围验收。用户确认后，整体 Review 才按以下四层独立裁决：

1. Core 行为 Review：在线、离线、gap、乱序、重复、失败、重启的数据库终态和 Emit 一致。
2. Gate Review：Phase1–4 80 Gate 与文字接龙 scoped 8 Gate 的分母、执行记录、失败和跳过逐项对账。
3. 跨层 Review：真实 Go HTTP/WS → Helix → Native SQLite → Tauri EventSink → Angular DOM 使用同一 eventId/eventSeq、channelId、viewer 和 corr 形成证据链。
4. 性能与边界 Review：批量 Effect 数满足上界，Go 零改动，Angular 零业务补算，legacy/compatibility 分支零新增且目标旧分支已移除。

Mock、Golden、Cargo Green、SQLite Green、HTTP 200、WebSocket 收帧、Tauri bus 收到事件和 DOM 截图分别只证明各自层级。只有用户授权整体 Review 且 required 层全部 Green，才能宣布“Phase1–4 与文字接龙整体不回退”；任何 `NOT_RUN`、`BLOCKED`、`PARTIAL`、skip 或分母不一致都阻止整体 Green。若用户暂不授权，则本次交付最多只能声明“实现与 focused review 完成，整体回归待用户确认”，不能扩大结论。

## Out of Scope

- 不修改 cses-im-server 的 Go handler、事务、event type、WebSocket 推送、Sync payload 或数据库 schema。
- 不新增 `update_channel` 兼容推送，不要求 Go 为客户端局部投影补发第二种业务事实。
- 不在 Angular/Tauri 增加未读计算、lastPost 猜测、消息顺序修复或直接 Sync HTTP 调用。
- 不把 IM 事件业务下沉到通用 `helix-core`。
- 不重新定义服务端 `channel_event.event_seq`、WebSocket transport seq 或已读业务合同。
- 不因撤回本地推导未读减一或前一条 lastPost；若产品需要该行为，应以新的服务端权威合同另立 Feature。
- 不实现文字接龙 Category 或 Acceptance 尚未完成的 4 Gate，不把 scoped 8/8 表述为 Phase5 12/12。
- 不借本次修复重构无关 Channel、Timeline、Search、Bot、Vote、Schedule 或权限领域。
- 不以迁移期双写、旧新 reducer 并存、silent fallback 或“缺字段时猜一个值”换取表面兼容。
- 不在本 Spec 阶段宣称真实环境已经 Green；只有实现后的分层验证报告可以作出该结论。

## Further Notes

本 Spec 收敛的是正确性与性能同一个问题：同一事件只编译一次、同一批次只提交一次、同一 viewer 成员态每频道只读回一次、同一 Dialog 只发布最终绝对值。减少重复路径后，性能收益来自状态空间和 IO 次数下降，而不是跳过持久化屏障或降低一致性。

实施阶段必须先删除或替换两类已知偏差，不得只在旁边新增“正确路径”：Sync type=1 写共享 `channel.unread_count` 并读共享 channel 行的路径；type=2 gap 的 pre-Persist 提前落库/提前 Emit 兼容路径。旧路径仍生产可达即视为未完成。

编码与 focused review 报告应明确列出：变更前基线、focused TDD、SQLite 集成、相关 Helix 测试、性能 Effect 计数、Go 零改动证明、客户端 production-boundary 检查和本次 diff review。完成该报告后必须询问用户是否启动整体 Review。

用户授权后的整体 Review 报告再列出：客户端 worktree 中阶段闸门规划文档及其 manifest 真源、Phase1–4 逐 Phase、文字接龙 scoped 8、真实双用户在线、断线离线、PongGap、重启 Hydration、跨层 event/corr 对账和最终独立裁决。报告只陈述已执行证据，缺失项保持阻断状态。

实现工作开始前，执行者应把本 Spec 拆成按依赖排序的测试与代码任务，并冻结统一 mutation compiler 的输入输出合同。任何需要改变 Go wire、引入客户端业务补算、恢复 legacy 双轨或改变既有产品未读/撤回语义的发现，必须停止实施并回到人工决策。
