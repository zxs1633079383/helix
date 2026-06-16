# Rule — Agent/Workflow 架构 review 收敛节律（强制行为）

> 根 [`CLAUDE.md`](../../CLAUDE.md) 的约束层展开（按需 Read）。配套：
> 台账 [`docs/review/workflow-review-ledger.md`](../../docs/review/workflow-review-ledger.md) ·
> 闸门 `scripts/review-gap.sh` · review workflow `Workflow({name:"architecture-drift-review"})`。
> 核心一句话：**自主改动每累计 5 个 commit，必须过一次架构 review 收敛，防偏移。**

---

## 1. 为什么（why）

agent / workflow 自主连续改代码，跑着跑着可能悄悄偏离架构（破 HX-C001..C008 不变量 / 违 GOAL §1 设计理念 / 全流程接缝变得不合理）而无人察觉——等发现时已积累大量偏移，回正代价指数级。本规则把「架构对齐」从「想起来才做」降级为**commit 距离触发的强制闸门**。

## 2. 不变量（强制）

```
gap = git rev-list --count <last_reviewed_commit>..HEAD     # 见 ledger
gap ≥ 5  →  🔴 强制：先派发 Workflow({name:"architecture-drift-review"})，review 通过 + 更新 ledger 后，方可继续 feature 开发
```

- **阈值 = 5 commit**（「临界点五个」）。
- `last_reviewed_commit` 真源 = `docs/review/workflow-review-ledger.md`。
- review 通过 → ledger `last_reviewed_commit` 改为本次 HEAD + 追历史行 + runId → gap 归 0。

## 3. 何时检查（触发点）

| 触发点 | 动作 |
|---|---|
| **开局新会话**（读完 ROADMAP/OPERATING 后）| 跑 `scripts/review-gap.sh`；退出码 1 → 本会话**第一件事**是派 review workflow，不先做 feature |
| **每次 commit 后** | 跑 `scripts/review-gap.sh`；命中 → 立即收敛，不攒着 |
| **准备派新 feature workflow 前** | 先 `scripts/review-gap.sh`；命中 → 先 review 后 feature |
| **会话收尾** | gap≥4 时提醒「下次开局须 review」 |

> `scripts/review-gap.sh` 退出码：0=ok / 1=强制 review / 2=ledger 异常。可挂 pre-push / Stop hook 兜底。

## 4. review workflow 查什么（架构不偏移 + 全流程合理）

`Workflow({name:"architecture-drift-review"})` 对 `<last_reviewed_commit>..HEAD` 的 diff 跑：

1. **HX-C001..C008 不变量**逐条（core 零运行时/零 schema、Send 边界、features additive、热路径 O(1)、FFI 边界、Effect 批处理、sync-wire 契约）。
2. **GOAL §1 设计理念 + §2 硬约束**（sans-IO / 确定性 / 三层依赖单向 / 双端边界）。
3. **全流程合理性**：收发(WS/HTTP) → 解析(parser) → cursor(gate/续拉) → 落库(schema/列对齐) 接缝是否仍自洽、有无死代码/绕过/临时 mock 残留。
4. 对每条疑似 drift **对抗证伪**，确认后给修复收敛动作。

输出：drift 判定（🟢无/🟡轻/🔴重）+ findings + 收敛 todo + 「ledger 更新到 commit X」指令。

## 5. 强制规则

- ❌ gap≥5 还继续派 feature workflow / 攒新改动 —— 必须先 review 收敛。
- ❌ review 通过却不更新 ledger `last_reviewed_commit` —— gap 永不归零 = 闸门失效。
- ❌ 把 `last_reviewed_commit` 手动跳到未真正 review 的 commit —— 闸门作弊。
- ✅ review 发现 🔴 重 drift → 先修复回正再更新 ledger（ledger 指向「已收敛」的 commit）。
- ✅ ledger `last_reviewed_commit` 被 rebase/squash 失联 → 脚本 exit 2 提示，手动校正到等价 commit。
