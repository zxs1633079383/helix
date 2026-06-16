# Agent Workflow Review Ledger — 架构不偏移收敛台账

> **规则**：[`.claude/rules/agent-review-cadence.md`](../../.claude/rules/agent-review-cadence.md)
> **闸门脚本**：`scripts/review-gap.sh`（算 HEAD 距上次 review 的 commit 距离）
> **review workflow**：`Workflow({name:"architecture-drift-review"})`

## 不变量（强制收敛）

```
gap = git rev-list --count <last_reviewed_commit>..HEAD
gap ≥ 10  →  🔴 强制：在 commit 检查点（非中途任务）派发 Workflow({name:"architecture-drift-review"}) 收敛后，方可继续 feature 开发
review 通过 → 把 last_reviewed_commit 更新为本次 review 的 HEAD + 追历史行 → gap 归 0
```

目的：任何 agent / workflow 自主改动累计**不超过 10 个 commit** 就必须过一次架构 review（HX-C001..C008 不变量 + GOAL §1/§2 设计理念 + 全流程合理性：收发→解析→cursor→落库），防止「跑着跑着偏离架构」无人察觉。**只在 commit 检查点判定，绝不中途任务触发**（不打断实现）；命中可自动派 drift-review（无需问用户，token 高性价比）。

## 当前状态

```
last_reviewed_commit: 3c5324f
last_reviewed_at: 2026-06-16
last_review_runId: wf_ee61a139-68b (architecture-drift-review，首轮)
threshold: 10
```

> 说明：2026-06-16 规则生效，基线锚点 `8c69f25`。首轮 review（gap=5 触发，阈值当时=5，已调至 10）核 `8c69f25..3c5324f`，🟡 轻 drift（2 条，非阻塞），ledger 推进到 `3c5324f`，gap 归 0。

## 待收敛 todo（drift-review 登记，择期修，非阻塞）

- [ ] **[medium] HX-C005 热路径回归**：`event_to_upsert_op`（channel.rs:296-353，6d83ad5 引入）WS 每事件 `serde_json::from_slice(raw)` 全量重解析 + ~14 次 String 堆分配；sync 路径 parse→reserialize(parser.rs:313)→parse 三趟。SQL 写仍真 O(1)（单 BatchUpsert ON CONFLICT，无 RMW），受 WS 帧率约束非 fan-out → medium。**修法**：A. 字段提取上移 parser，EventEnvelope 携 owned typed 视图（消除 reserialize）→ **与 B5（Post map→message 内容落库）同源，建议合并到 B5 轮**；B. 列名 `&'static str`；C. channel.rs:293 注释诚实化（SQL O(1) 但分配非零）；D. 承压实测 alloc 数。
- [ ] **[low] 双 cursor schema 占位**：`schema.rs:131` 建 `channel_event_cursor` 死表（零读零写），运行时实际写 `_helix_monotonic`。原判「污染 core §1.6」已证伪（core 零业务 schema；channel_event_cursor 在 helix-im 属正当位置）→ 实为 helix-im 内 cursor 单一真源自洽瑕疵。**修法**：A.（推荐）从 IM_SCHEMA 删 `channel_event_cursor`，M2 再引入并写迁移 `_helix_monotonic` 已有水位步骤；或 B. 保留+注释不变量「_helix_monotonic 唯一真源」+ grep gate 禁 SELECT channel_event_cursor。

## 历史

| 日期 | review 到 commit | 区间(from..to) | workflow runId | drift 判定 | 关键发现 / 收敛动作 |
|---|---|---|---|---|---|
| 2026-06-16 | 8c69f25 | — (基线) | — | 🟢 baseline | 规则生效锚点；无 review，仅设基线 |
| 2026-06-16 | 3c5324f | 8c69f25..3c5324f | wf_ee61a139-68b | 🟡 轻 drift | 2 条真 drift（8 报告去重）：[medium] event_to_upsert_op 热路径二/三次 serde + per-event alloc（违 HX-C005，6d83ad5 引入）；[low] channel_event_cursor 死表占位 vs 运行时 _helix_monotonic。均非阻塞、无正确性崩塌 → 登记「待收敛 todo」择期修，ledger 直接推进 |
