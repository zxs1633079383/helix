---
id: HX-C012
title: 实质任务须开局写 🎯 EXPECT，禁事后补记（堵 DRIFT-PRED 永空 audit 后门）
status: active
severity: medium
scope: helix
created: 2026-06-21
source: 2026-06-21 用户明确拍板建卡 + 同根 3（finalize DONE 事后补记 + a80cc49 C++ smoke DONE 缺 EXPECT + 建卡当会话「修复候选+f5」任务自身漏写 EXPECT 自指复发）；drift-review wf_fd406543-c76 候选回放闸门判 HOLD（同根升 3 后基础部分补齐）
recurrence_count: 3
trigger_paths:
  - "SESSION.md"
  - "crates/**/tests/**"
  - "crates/**/benches/**"
  - "scripts/**"
trigger_keywords:
  - "EXPECT"
  - "DONE"
  - "DRIFT-PRED"
  - "harness"
  - "smoke"
  - "CI"
related:
  - HX-C009
  - HX-C011
graduates_to:
  - AGENTS.md
---

# HX-C012 — 实质任务须开局写 🎯 EXPECT，禁事后补记

## §1 触发场景
开始任何达「实质任务门槛」的工作**之前**（AGENTS.md §4.8 门槛）：
- ≥3 文件 / 净增删 ≥50 行 / 跨 crate / 改 port 或 driver / 迁移 / 不可逆操作
- **以及**：新增 test / CI / smoke / harness（≥50 行或含**新源文件**）——执行者最易把「加测试/加脚本/加 harness」误当豁免类，这正是历史漏点。

豁免：typo / 单行 / 纯格式（与 §4.8 一致）。

## §2 背景（why）——同根 3（含自指）+ 用户拍板
| # | 日期 | 现象 | 根因 |
|---|---|---|---|
| 1 | 2026-06-18 | finalize 收尾 DONE 缺配对 🎯 EXPECT（事后补记） | 把「收尾/收口」当非实质，DRIFT-PRED 一度悬空 |
| 2 | 2026-06-18 | a80cc49 C++ 直连冒烟 harness DONE（130 行 / 6 文件 / smoke.cpp+CMakeLists 两新源文件，三重命中门槛）缺配对 EXPECT | 把「加 test/harness/脚本」当 EXPECT 豁免类 |
| 3 | 2026-06-21 | **建本卡当会话**「修复候选+f5」任务（建 HX-C012 + 改 6 文件跨 docs/test）自身**未开局写 EXPECT** | 自指复发——连建卡者执行实质修复时都漏，证此坑高频；如实记录而非补假 EXPECT（守 §3） |

**核心**：没有开局 EXPECT，收尾就**没有对账基线**——`⚡ DRIFT-PRED` 结构性永空，drift-review 失去 prediction-error 这根进料管（§4.8「偏离判定不自产自判」要堵的 audit 后门）。EXPECT 事后补记 ≈ 自评自判，等于后门没堵。

## §3 Required / Forbidden
```
✅ Required：
   - 达门槛任务**开局即**写可证伪 🎯 EXPECT 进 SESSION §0（含 ≥1 可证伪断言：改动文件集 /
     数值阈值 / 命名不变量 / 命令 pass-fail）。
   - test / CI / smoke / harness 性质任务（≥50 行或含新源文件）**不豁免**——同样开局写 EXPECT。
   - 收尾 ✅ DONE 必须有时间上更早的配对 EXPECT 作对账方；命中即对账，偏离追 ⚡ DRIFT-PRED。
```
```
❌ Forbidden：
   - 事后补记 EXPECT（DONE 之后才补 = 失去对账意义，DRIFT-PRED 悬空后门）。
   - 把「加测试 / 加脚本 / 加 harness / 收尾收口」当 EXPECT 豁免类。
   - ✅ DONE 无配对 🎯 EXPECT（孤儿 DONE = audit trail 断裂）。
```

## §4 Verification
```bash
# 每个 SESSION §0 的 ✅ DONE 应有时间更早的配对 🎯 EXPECT（同任务关键词）。
# 列出 DONE 任务关键词，人工核对其上方/历史是否有配对 EXPECT：
grep -nE '^(🎯 EXPECT|✅ DONE)' SESSION.md
# 新增 harness / smoke / CI（含新源文件）的 commit，回查其 SESSION §0 是否开局有 EXPECT：
git log --oneline --diff-filter=A -- 'crates/**/tests/**' 'scripts/**smoke**' 'docs/harness/HX-C*.md'
```
> 审查准则：每个 ✅ DONE 问一句「它开局写 EXPECT 了吗？」——没有 = audit 后门，补流程（非事后补 EXPECT 掩盖）。

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-18 | daf44fe（finalize 收口） | DONE 缺配对 EXPECT，事后补记 | 收尾被当非实质任务 |
| 2026-06-18 | a80cc49（C++ smoke harness） | DONE 缺配对 EXPECT | 加 test/harness 被当豁免类 |
| 2026-06-21 | 本会话「修复候选和f5」（3418ac7/2664cfd） | 建 HX-C012 + f5 修复（6 文件）实质任务自身缺开局 EXPECT（**自指**） | 实质修复任务开局 EXPECT 仍易漏，连建卡当下都犯——如实记录补 recurrence→3 |

## §6 关联
- 兄弟：**HX-C009**（预期对账闭环完整性：EXPECT 可证伪 + 偏离不自产自判）——C012 是其**前置**：先保证 EXPECT **存在且开局写**（audit trail 完整），C009 再保证其**内容可证伪 + 对账独立裁定**。
- 兄弟：HX-C011（测试可证伪性）——同属「测了/写了 ≠ 证明了/对账了」家族。
- 上游：`AGENTS.md §4.8`（预期对账 L1 决策原子，晋升候选——把「test/CI/harness ≥50 行或新源文件须开局 EXPECT」显式补进 §4.8 触发清单）。

## §7 历史与演进
- 2026-06-21 建卡。**诚实记录张力**：本候选在 drift-review `wf_fd406543-c76` 被回放闸门判 **HOLD**——历史 trap（log.md：HX-C005 cursor format / HX-C008 sync 缺 type / HX-C010 SipHash 迭代序）均为**代码级约束**违反，本候选是**流程治理缺口**（漏写 EXPECT），性质不同源，历史回放既挡不住这次、也无法验证不误伤既往成功 → 回放无支撑。
- 但**用户 2026-06-21 明确拍板建卡**独立满足建卡三铁律之一（「用户明确」），且同根已 ≥2 次逼近 ≥3 阈值。属流程/元约束类卡（先例 HX-C009 同族，非代码级 grep 约束）。
- 2026-06-21 **自指复发（同根 2→3）**：建卡当会话执行「修复候选+f5」实质任务（建 HX-C012 + f5 修复，6 文件跨 docs/test）时自身漏写开局 EXPECT。这**部分缓解回放闸门 HOLD**——同根达 3 阈值（虽性质仍属流程治理而非代码级 trap）；如 HX-C011「建卡当 diff 即抓到违反」，自指复发是约束高频性的最强证据。**如实记录此漏写，非补假 EXPECT（守 §3 Forbidden）。**
- 后续：晋升候选 = 把规则 inline 进 `AGENTS.md §4.8` 触发清单。
