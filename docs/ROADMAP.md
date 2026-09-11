# Helix 当前 Roadmap

> 本文是当前活账。目标常量见 [`GOAL.md`](./GOAL.md)，架构决策见
> [`harness/decisions.md`](./harness/decisions.md)，MessageV3 实施真源见
> [`specs/047-im-event-gate-inventory`](../specs/047-im-event-gate-inventory/)。
>
> Snapshot：2026-09-11（代码基线 `f4def049`；版本/静态入口对账，历史验收未重跑）。旧 Frame/Surface 报告属于历史证据，不自动继承为当前 Green。
> `verification.md` 与 `evidence/p7-final-validation.txt` 记录的是固定历史 evidence window；当前 checkout 的
> `.v1` 被 Git 忽略且未生成，`review-ledger.json` 仍为 `verifying`，因此下文不能单独作为当前 final Green。

## 1. 当前结论

`helix-im` 已从旧呈现套壳与旧 V1 compatibility 硬切到 MessageV3 Gate Event。
唯一业务出站合同为 `Effect::Emit { event: DomainEventBytes }`，其中 bytes 载荷为 `{event,data}`；driver 只批量交付 bytes，平台不重建业务 reducer。

本轮分母为 Spec 047 冻结的 30 Gate。G-03 与 G-17 至 G-27 为 `OUT_OF_SCOPE`。本版本登记了
Helix 生产接码、真实回放测试与 V2/V1 物理下线；历史 evidence window 曾生成被忽略的 `.v1`
canonical JSON，但当前 checkout 需重新生成并绑定当前 revision 后才能复用；
PC WDIO、Mobile GTest、Go 修改与跨仓 fixture 同步明确延期。

当前状态：

- Constitution 2.0 与 ADR-017 已确立 MessageV3 唯一真源；
- Spec 047 的 P0-P7、T001-T090 已登记完成，30 Gate 均为 `MIGRATED`；R4 仍以固定历史 evidence window 记录，当前
  `review-ledger.json` 阶段为 `verifying`，不能写成已 close；
- 历史 evidence window 的 deterministic、contract 与 Rust driver integration 记录为 `PASS`，30 Gate × 4 canonical JSON
  的当前 checkout 产物状态为 `NOT_GENERATED`，不能把历史 hash 代替当前产物；
- PC/Mobile canonical consumer、真实宿主、真实 Go/WS 与双用户证据仍为
  `NOT_SYNCED_BY_SCOPE` / `NOT_RUN`，不得据此宣称三端 TGCR Green。
- 四个公共包的当前 Cargo manifest 版本统一为 `0.1.18`；workspace 内消费者以 Cargo manifest 的
  精确依赖为准。
- 独立的 Native Phase1-5 真实链 runner 当前可执行分母为 83 Gate（32/16/15/12/8）；
  五个 `NOT_APPLICABLE` 与四个 `NOT_IMPLEMENTED` Gate 从 runner 选择集排除。变更前的
  92 Gate 与后续 83 Gate 的运行结果均须绑定各自候选版本与 evidence window；它们不改变 Spec 047 的
  30 producer Gate 分母，也不能替代 PC/Mobile/Web 的 TGCR 证据。

## 2. 不变平台地基

| 能力 | 状态 | 本轮边界 |
|---|---|---|
| runtime-agnostic core | 保留 | core 继续只拥有 Tick、Effect、ports 与 Module host |
| shared host | 保留 | SQLite、HTTP、WS、timer 与 engine loop 不复制业务逻辑 |
| native / FFI / Web driver | 保留 | 原样交付 `DomainEventBytes`，不包装业务 Frame |
| tenant SQLite | 保留 | PersistOk 后 MessageV3 终态真源 |
| Go wire parser | 按需保留 | 现役 WS action 保留，只删除旧 alias/fallback/normalize |
| 旧呈现业务层 | 已退役 | Frame/Surface/Projector 与旧 V1 compatibility 已物理删除；历史文档进入 allowlist |

## 3. P0 至 P7 实施活账

### P0：治理、删除清单与公共基础

- [x] 更新 GOAL、ADR、ROADMAP、AGENTS 与 HX-C018。
- [x] 建立精确 30 Gate migration manifest，编码前登记生产 owner 和 V2/V1 删除引用。
- [x] 建立 MessageV3 event builder、PersistOk 公共红测与 artifact writer。
- [x] Node 生成器只运行 Rust 测试、索引并校验 schema/hash。
- [x] registry/checker 对账 30 Gate、七域映射与双 ledger 单活跃。

停止条件：治理真源一致；删除清单无未知 owner/空数组/TODO；公共红测先失败再 Green。

### P1：G-15a 与 G-01

- [x] G-15a 建群双视角：owner/invitee summary、roster 与角色。
- [x] G-01 文本发送：sender/receiver 同 server identity，sender unread=0，
  receiver 绝对 unread+1，双方更新 `lastPost`。

停止条件：两 Gate 的真实回放、四份 JSON、专属旧链删除与聚焦回归全部通过。

### P2：Post 变更

- [x] G-02、G-03a、G-04、G-04a、G-05、G-06、G-07、G-14、G-14a。
- [x] 覆盖文档消息、失败重试、撤回/恢复、转发、reaction、加急与消息回执。

停止条件：每 Gate 严格 test-red -> event-green -> legacy-removed -> migrated，不跨 Gate 双写。

### P3：Schedule

- [x] G-08 创建定时消息。
- [x] G-09 取消定时消息。

停止条件：创建者私有终态、恢复一致且不向 receiver 制造普通消息。

### P4：Timeline 与 Read

- [x] G-10、G-10a、G-10b、G-10c、G-10d、G-11、G-11a、G-12、G-13。
- [x] 保持 bounded window、稳定 anchor、分页 token、定位与 viewer-local read。

停止条件：无 Surface/Frame 导航 directive，事件只表达已提交业务事实与明确查询结果。

### P5：Channel、Topic、Member 与 Settings

- [x] G-15、G-16、G-16b、G-16c。
- [x] G-16b 使用独立 `channel-member` domain / `channel_member` Rust 测试包。

停止条件：生命周期、roster、角色与群属性按 viewer 收敛，专属旧 channel/member Surface 删除。

### P6：Sync 与 Recovery

- [x] G-S1、G-S2、G-S3、G-S4。
- [x] 在线 event 与离线恢复输出同一 MessageV3 终态，cursor 不先于数据提交。

停止条件：hello、notify、pong gap 与陌生频道补全均通过 PersistOk 和恢复回放。

### P7：共享旧链物理清除

- [x] 删除旧呈现协议、Projector、旧 V1 compatibility 与相关 fixtures。
- [x] 当前生产、测试、Matrix 和合同文档的退役业务引用为 0；历史归档走显式 allowlist。
- [x] 生成 30 Gate × 4 canonical JSON，重复运行 byte/hash 稳定。
- [x] 运行 helix-im 全量、core、native/FFI/Web check 与 Web WASM build。
- [x] R4 记录真实证据；PC/Mobile 保持 `NOT_SYNCED_BY_SCOPE` 和 `NOT_RUN`。

停止条件：30 Gate 全部 `MIGRATED`，Helix 零旧业务引用，R4 PASS。未完成任一条件不得写
`CLOSED`。

## 4. 证据等级

| 等级 | 能证明 | 不能替代 |
|---|---|---|
| deterministic | Tick/Effect/PersistOk/Emit 顺序 | driver 或 UI |
| contract | schema、字段、hash、Gate 集合 | runtime |
| integration | crate/driver 接线 | 双用户真实链 |
| runtime | 当前宿主实际执行 | 另一平台 |
| dual-user | sender/receiver 同窗口传播 | 离线恢复 |
| canonical | producer 来源与 byte-exact mirror | 所有运行态 |

任一缺失面写 `BLOCKED`、`NOT_RUN` 或 `NOT_SYNCED_BY_SCOPE`，不借其他等级漂绿。

## 5. 工作树与提交边界

- 目标发布分支：`reactor/MessageV3-helix`；本轮运行分支、upstream、dirty 资产与 worktree 隔离记录在根 `SESSION.md`，
  不把某一运行 worktree 的分支/未跟踪文件写成所有 checkout 的当前事实。
- 源 checkout 若存在未跟踪 `artifacts/` 或私仓 Prompt，文档对齐不纳入；具体文件以本轮 `git status` 为准。
- 每个可独立验收的 Task/Phase 聚焦验证后立即 Conventional Commit，不自动 push。
- Rust 编码全程遵守 Cargo 2-3 次纪律；不做改一行跑一次的编译循环。
