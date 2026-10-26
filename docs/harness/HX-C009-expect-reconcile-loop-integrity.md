---
id: HX-C009
title: 预期对账证据完整性——EXPECT 可证伪 + DRIFT-PRED 如实记录（非阻塞）
status: active
severity: major
scope: helix
created: 2026-06-17
source: 2026-06-17 会话——§4.8 预期对账试点;独立核查抓到 B5「声明加扫 DRIFT-PRED 但脚本未实装」+ 漏洞7「自产自判 GIGO 后门」;用户明确要求沉淀
recurrence_count: 2
trigger_paths:
  - "AGENTS.md"
  - "SESSION.md"
trigger_keywords:
  - "DRIFT-PRED"
  - "EXPECT"
  - "预期对账"
  - "§4.7"
related:
  - HX-C008
graduates_to:
  - AGENTS.md
inline_target:
---

# HX-C009 — 预期对账证据完整性

## §1 触发场景（Trigger）

改 AGENTS.md §4.7 或 SESSION；问「EXPECT 怎么写 / 偏离怎么记录 / 这套是否会阻断开发」。grep：`DRIFT-PRED`、`EXPECT`、`预期对账`。

完整方法论 → [`crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md`](../../crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md)。

## §2 背景（why）

2026-06-17 落地「预期对账」：`🎯 EXPECT` → `✅ DONE`，偏离时写 `⚡ DRIFT-PRED`。2026-07-13 用户拍板移除按 commit 数触发的强制 review；本卡只保留轻量证据记录，不参与 feature、commit 或 push 的阻塞判定。

- **EXPECT 可证伪**：具体文件、命令或行为断言能让后续读者核对计划与结果。
- **偏离如实记录**：实际结果与 EXPECT 不同就写 `⚡ DRIFT-PRED`，作为排障和复盘线索。
- **不冒充闸门**：SESSION 记录不能因为缺失、模糊或历史未收敛而阻断当前开发。

> 核心：它是一份可检索的工作日志，不是审批系统。

## §3 Required / Forbidden

```text
# ✅ EXPECT 必须可证伪（AGENTS.md §4.7）：至少 1 条具体可核断言
🎯 EXPECT 只动 parser.rs + sync_session.rs; cargo check -p cses-helix-core 过; 不新增 HX-C 违反 | 赌注 中 | 可逆 是

# ✅ 收尾写 DONE；实际偏离时紧跟一条 DRIFT-PRED
# ✅ 缺失或历史未收敛只影响证据质量，不阻断开发、提交或推送
```

```
❌ 🎯 EXPECT "我觉得能行 / 应该没问题 / 大概顺利"(不可证伪 → DRIFT-PRED 永空)
❌ 用 EXPECT/DONE 是否完整作为 feature、commit 或 push 的阻塞条件
❌ 在 SESSION 写入 token、Cookie、密码或真实业务敏感内容
```

## §4 Verification

- 规则存在：`grep -nE "DRIFT-PRED|可证伪|不阻断" AGENTS.md` 必须命中。
- 记录可读：`grep -nE '^🎯 EXPECT|^✅ DONE|^⚡ DRIFT-PRED' SESSION.md` 能读取近期对账记录。
- 非阻塞口径：AGENTS.md 明确该机制不阻断 feature、commit 或 push。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-17 | B5 修复 `0f569b0` | §4.8 声明"加扫 DRIFT-PRED",workflow 脚本 0 处提及 SESSION | 声明停在文档散文,未落可执行接线 |
| 2026-06-17 | 漏洞7 修复 `ea0df41` | 同一 agent 自产 EXPECT 又自判偏离,可模糊化使惊讶永空 | 进料口无可证伪要求 + 偏离裁定未移出生产者 |
| 2026-07-13 | `codex/web-w0-development` | commit 数与全仓行数检查阻断 Web W0 开发 | 用户拍板删除强制 review/行数闸门，EXPECT 退回非阻塞证据记录 |

## §6 关联

- 上游：AGENTS.md §4.7（非阻塞预期对账）。
- 方法论长文:`crates/helix-im/docs/AI-agent-自主进化闭环-从哲学到工程.md`(哲学 + 工程 + 本案例)。
- 兄弟 harness:HX-C008(同为"行为/契约不可凭脑补,须实证核对"——B5 与 HX-C008 的 B3 同根:抽象脱离真源)。
- 长期规则：`AGENTS.md`（预期可证伪、结果有证据、记录不阻断开发）。

## §7 历史与演进

- drafting → active:2026-06-17 第一次命中(B5 + 漏洞7,同会话两修)。
- 2026-07-13：移除强制 review 接线；保留 EXPECT/DONE/DRIFT-PRED 作为非阻塞工作日志。
