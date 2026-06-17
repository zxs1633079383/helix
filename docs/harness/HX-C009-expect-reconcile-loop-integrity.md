---
id: HX-C009
title: 预期对账闭环完整性——DRIFT-PRED 须真接入 drift-review（非散文）+ EXPECT 可证伪 + 偏离裁定不自产自判
status: active
severity: major
scope: helix
created: 2026-06-17
source: 2026-06-17 会话——§4.8 预期对账试点;独立核查抓到 B5「声明加扫 DRIFT-PRED 但脚本未实装」+ 漏洞7「自产自判 GIGO 后门」;用户明确要求沉淀
recurrence_count: 1
trigger_paths:
  - ".claude/workflows/architecture-drift-review.workflow.js"
  - "CLAUDE.md"
  - "AGENTS.md"
  - "SESSION.md"
trigger_keywords:
  - "DRIFT-PRED"
  - "EXPECT"
  - "预期对账"
  - "expect-audit"
  - "replay_gate"
  - "回放闸门"
  - "自评自判"
  - "§4.8"
related:
  - HX-C008
graduates_to:
  - ~/.claude/rules/common/agents.md
inline_target:
---

# HX-C009 — 预期对账闭环完整性

## §1 触发场景（Trigger）

改 `architecture-drift-review.workflow.js`;改 CLAUDE.md / AGENTS.md §4.8;问「预期对账怎么跑 / DRIFT-PRED 接到哪 / EXPECT 怎么写 / 这套会不会被绕过」;任何声称"把某信号接入了某 workflow / review"的改动。grep:`DRIFT-PRED`、`expect-audit`、`replay_gate`、`预期对账`。

完整方法论 → [`crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md`](../../crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md)。

## §2 背景（why）

2026-06-17 落地 §4.8「预期对账」(L1 决策原子:`🎯 EXPECT` → `✅ DONE` 对账 → 偏离写 `⚡ DRIFT-PRED`,喂 §4.7 drift-review)。两个真实事故定下本卡三条不变量:

- **B5(声明 ≠ 实装)**:CLAUDE.md §4.8 写"drift-review 触发时加扫 DRIFT-PRED",但 `architecture-drift-review.workflow.js` 脚本**0 处提及 SESSION**——独立 grep 核查发现是**悬空引用**:接了一条管子,源头没接。文档散文不等于可执行接线。
- **漏洞7(自产自判 GIGO 后门)**:写 `EXPECT` 和判"是否偏离"是同一执行 agent → 可把预期写模糊使 `⚡ DRIFT-PRED` 永远为空。**下游裁判再严,空输入对账永远 PASS**——垃圾进垃圾出,不是 review 能补的。

> 核心:一个"会进化的闭环",最脆弱的不是裁判算法,是**进料口**。进料口若可被生产者自产自判 + 空话化,整条链是空转的摆设。

## §3 Required / Forbidden

```js
// ✅ DRIFT-PRED 必须被脚本真读(可 grep 证明),不是文档里写写
// architecture-drift-review.workflow.js / Scope 阶段:
//   grep -n '⚡ DRIFT-PRED' SESSION.md  → 填 drift_pred
//   并有专门 predReview lens 消费它 + replay_gate 取真实 docs/harness/log.md

// ✅ EXPECT 必须可证伪(§4.8):至少 1 条具体可核断言
🎯 EXPECT 只动 parser.rs + sync_session.rs; cargo check -p helix-core 过; 不新增 HX-C 违反 | 赌注 中 | 可逆 是

// ✅ 偏离裁定移出生产者:由 drift-review 独立 expect-audit lens 反向查
//   ① EXPECT 可证伪性 ② DONE 偏离 EXPECT 却没记 DRIFT-PRED(惊讶吞没)
```

```
❌ 在 CLAUDE.md / 规则文档写"workflow 加扫 X",但 workflow 脚本不含对应读取代码(声明 ≠ 实装)
❌ 🎯 EXPECT "我觉得能行 / 应该没问题 / 大概顺利"(不可证伪 → DRIFT-PRED 永空)
❌ 写 EXPECT 的 agent 自己判"没偏离、不记 DRIFT-PRED"就收口(自产自判)
❌ 候选教训直接建 HX-C 卡,不过回放闸门(拿候选回放 log.md 历史 trap)
```

## §4 Verification

- grep gate(接线真实性,挡 B5 回归):
  `grep -nE "DRIFT-PRED|expect_done|replay_gate" .claude/workflows/architecture-drift-review.workflow.js` 期望**命中**(Scope grep + lens + 闸门都在);为空 = 接线退化回散文。
- grep gate(漏洞7 回归):
  `grep -c "可证伪\|不自产自判" CLAUDE.md AGENTS.md` 期望各 **≥1**;`expect-audit` lens 必须存在于 workflow。
- 语法:`node --check .claude/workflows/architecture-drift-review.workflow.js` PASS。
- 镜像:`diff -q CLAUDE.md AGENTS.md`(§4.8 两文件逐字一致)。
- 阈值一致:全仓 review 节律 gap≥10(CLAUDE/AGENTS/cadence/README/`review-gap.sh` THRESHOLD),无 gap≥5 残留(业务 sync 的 `gap>5000` 无关)。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-17 | B5 修复 `0f569b0` | §4.8 声明"加扫 DRIFT-PRED",workflow 脚本 0 处提及 SESSION | 声明停在文档散文,未落可执行接线 |
| 2026-06-17 | 漏洞7 修复 `ea0df41` | 同一 agent 自产 EXPECT 又自判偏离,可模糊化使惊讶永空 | 进料口无可证伪要求 + 偏离裁定未移出生产者 |

## §6 关联

- 上游:CLAUDE.md §4.8(预期对账)+ §4.7(drift-review 收敛节律);`.claude/rules/agent-review-cadence.md` §4。
- 方法论长文:`crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md`(哲学 + 工程 + 本案例)。
- 兄弟 harness:HX-C008(同为"行为/契约不可凭脑补,须实证核对"——B5 与 HX-C008 的 B3 同根:抽象脱离真源)。
- 全局 rule(待晋升):`~/.claude/rules/common/agents.md`(裁判不共享提议者先验 / 独立验证才算 DONE)。

## §7 历史与演进

- drafting → active:2026-06-17 第一次命中(B5 + 漏洞7,同会话两修)。
- 待办(2 周校准后复核):trivial-gaming(具体但低风险 EXPECT 钻空)是否需加"EXPECT 动工前时间戳锁死";有效率达标则 §4.8 机制晋升全局 CLAUDE.md,本卡随之 graduates_to `agents.md`。
