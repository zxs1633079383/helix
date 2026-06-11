---
id: HX-CXXX
title: <一句话说清不许做什么 / 必须怎么做>
status: drafting        # drafting | active | merged | deprecated
severity: blocker       # blocker（阻断合并/三栖碎）| major（破不变量）| minor
scope: helix
created: 2026-MM-DD
source: <2026-06-08 review / logs 行号 / Spec 拍板>
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/src/**"
  - "crates/helix-im/src/**"
  - "crates/helix-driver-*/src/**"
trigger_keywords:
  - "<grep 友好关键词>"
related:
  - HX-CXXX
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
inline_target:          # merged 后指向 rules/ 锚点
---

# HX-CXXX — <title>

## §1 触发场景（Trigger）
什么情况下这条契约要被加载？给 grep 友好的关键词 / 文件模式 / 用户提问关键词。

## §2 背景（why）
为什么存在？踩了什么坑？必须有具体 commit / review 发现 / 时间戳，不许写"为了代码质量"。

## §3 Required / Forbidden
```rust
// ✅ 必须这么做
```
```rust
// ❌ 不许这么做
```

## §4 Verification
- grep / cargo 命令：`...` 期望结果
- CI gate：哪个 target / script 卡这条
- 单测：哪个 test 覆盖

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-MM-DD | <id/sha> | <一句话> | <一句话> |

## §6 关联
- 上游：CLAUDE.md §X / rules/runtime-agnostic-core.md §Y
- 兄弟 harness：HX-CNN
- Dioxus / cses 对照（如有）

## §7 历史与演进
- drafting → active：<日期 + 第一次命中>
- (merged 时) → rules/：<inline 锚点>
