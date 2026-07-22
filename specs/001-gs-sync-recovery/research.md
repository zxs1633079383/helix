# Research: GS01-GS04 同步恢复

## R1 - V2 迁移基线

**Decision**: 将当前 `interaction_frame.rs` 的 V1 八 Surface、泛型 `SurfaceDataVm` 下行视为迁移起点；GS01-GS04 直接产出 typed `InteractionFrame v2`，包含第九个 `message.composer` Surface，而不是在 V1 增补 raw event 兼容层。

**Rationale**: 当前 Rust 与 Angular contracts 都列出八个 Surface，且客户端已具备 schema v2 常量、Frame decode、按 slot 暂存与单 signal swap 的原子 cache 基础。V2 目标合同已冻结九 Surface 和 composer ownership。继续在 raw `im:*` 上叠一个 reducer，会让 client 同时消费两个业务真源。

**Alternatives considered**:

- 保留 raw emit 并在 UI 侧合成 V2：拒绝。会让 cursor、排序、未读、权限在 renderer 重新解释。
- 先只迁 dialog 不加 runtime/composer：拒绝。同步中状态与陌生频道可发送性会脱离同一提交/Frame 边界。

## R2 - 同步提交与 cursor 的原子性

**Decision**: 每个 `SyncBatchFacts` 用一笔 `Effect::PersistAtomic` 写入事实、账本、coverage 和 cursor；`SurfaceComposer` 只处理 `PersistOk`。

**Rationale**: 当前 `sync/flow.rs` 已把事实和 `channel_event_cursor` 放入 correlated persist，并延后 cursor/投影发布；host `atomic_write` 已在一个 SQLite transaction 中拒绝 read op。将 ledger/coverage 放入同一写集即可把“已提交水位”升级为可审计边界。

**Alternatives considered**:

- 先写 cursor，后补 message/ledger：拒绝。进程中断会把缺失数据伪装为已恢复。
- `PersistFire` 后立即 compose：拒绝。它没有可关联的 commit receipt，不能作为 UI 完成证据。

## R3 - 本地 channel event 对账证据

**Decision**: 引入 `channel_event_ledger`（逐 event 应用证据）和 `channel_sync_coverage`（逐原子批次覆盖范围）；DDL 追加到 `helix-im::schema::IM_SCHEMA`，`helix-im` 构造其 `StorageOp`，driver 只执行已给定 DDL 与通用写操作。

**Rationale**: 已观察的本地库有 `channel_event_cursor`，但没有物理 `channel_event`。远端 PG 有 `channel_event` 与 head。ledger 的复合唯一键消除重复，coverage 关联 `from_seq/to_seq`、event count、payload/事实摘要 hash 和 correlation，才能说明 cursor 不是孤立数字。

**Alternatives considered**:

- 只用 cursor：拒绝，无法满足 FR-005。
- 落远端 event 原始 JSON：拒绝。扩大本地隐私/存储面且 renderer 不需要；本 Feature 只保存可审计最小元数据和 canonical hash。
- 让 driver 识别 `channel_event` 的语义：拒绝，违反 driver 不懂业务 schema 的边界。

## R4 - 远端权威水位比较

**Decision**: 以当前身份和 channel scope 的本地 committed cursor 对比远端 authority head；本地低于远端拉缺口，本地等于远端不重放，本地高于远端或缺口不可连续进入 authority reload/人工介入路径。

**Rationale**: `sync/flow.rs` 已有 continuation、too-long、terminal/gap 防护。恢复完成只能依据远端 head 和成功 commit，不依据 WS 已连接或 HTTP 200。

**Alternatives considered**:

- 连接即声明 ready：拒绝。WS connected 不是数据一致。
- 客户端自行填补漏序：拒绝。会产生非权威 event/history。

## R5 - SurfaceComposer 的影响判定

**Decision**: composer 接收 committed fact change set 与 attach registry，按 contract 输出最小关联 bundle：用户 dialog/unread/runtime 同帧；已 attach timeline/thread 仅窗口命中时更新；已打开频道的 summary 与 composer 同帧。

**Rationale**: 事实影响 UI 的关系是业务规则，应在 `helix-im`；但没有 attach 的视图不应被强制创建，避免无界 VM 重建。

**Alternatives considered**:

- 每次同步重建九个 Surface：拒绝。无谓 O(n) 与 UI 抖动。
- UI 从 message event 推断 channel/composer：拒绝。权限和 hydration 顺序会漂移。

## R6 - ActionBinding 和陌生频道

**Decision**: `message.composer@Channel(channelId)` 独立拥有 send action binding。hydration 未提交时 binding 缺席或 `enabled=false`；summary 与 enabled composer 同一 Frame 产生。

**Rationale**: actionRef 稳定性依赖 scope/epoch/revision，不能从 timeline row 或 summary action 重用。独立 composer VM 让未来消息类型只扩展 payload tagged union，而不迫使所有 Row/VM 加字段。

**Alternatives considered**:

- 把 send action 放 timeline VM：拒绝。timeline window 变动会无关地失效编辑器 actionRef。
- 前端根据 `channelId` 自造发送 command：拒绝。绕过权限/epoch fail-closed。

## R7 - Frame Cache 与 directive/ack

**Decision**: 所有 mutation 先 decode、校验 scope/epoch/revision 并在 staging map 中完成；任一 attached slot gap/stale 则全 Frame 不交换、为相应 scope 请求 resync。成功时单次 version swap，之后按顺序执行 directive 并发送 ack。

**Rationale**: 现有 `SurfaceRuntime` 已遵循 staged state + one `bump()` 的设计；V2 将 runtime 也作为同事务的 mutation，避免同步提示与业务 VM 在不同版本显示。

**Alternatives considered**:

- 收到 mutation 即逐 slot set：拒绝。用户会看见同一业务事实的半帧。
- directive 在 cache commit 前执行：拒绝。焦点/提示可能指向尚未存在的渲染状态。

## R8 - 验收证据

**Decision**: 每个可签收 run 都收集 deterministic Rust、PC DOM、readonly SQLite/PG 对账、correlation hops 四面证据；mobile 用相同 Frame fixture 做 semantic apply parity，直到真实 mobile UI gate 可运行。

**Rationale**: 单元测试不能证明本地表和远端 event 对齐，DB 对账不能证明 renderer 可见；四面互补且全部只读。

**Alternatives considered**:

- 只跑 `cargo test`：拒绝。缺 UI 与真实证据。
- 直接改 SQLite/PG 把结果调成一致：拒绝。破坏证据可信度，违反 FR-011。

## R9 - 30 分钟进度

**Decision**: 在实施 runbook 中定义人工/工作流的 30 分钟状态行，不创建 cron 或无人值守写入器。

**Rationale**: 项目规则禁止为可追踪工作建立定时轮询；进度报告是透明度合同，不是业务功能。

**Alternatives considered**:

- 定时任务自动宣告完成：拒绝。没有验证结果的计划/日志不能替代完成证据。
