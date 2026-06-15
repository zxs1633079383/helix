# Helix Harness 跨卡复现日志

> 每次再次踩到任一 HX-C{NNN} 的坑 → 这里追一行 + 对应卡 §5 +1 + frontmatter recurrence_count +1（同 commit 三同步）。

| 日期 | 卡 | session/commit | 现象 | 处置 |
|---|---|---|---|---|
| 2026-06-08 | HX-C001..C007 | 多专家 review wf_281205c2 | 首批 7 条约束建立（3 blocker + 5 major review 发现 + Dioxus 对照 + runtime-agnostic 规约）| 卡片创建 |
| 2026-06-15 | HX-C005 | a0595607 review / commit c6a655a | a0595607 重构把 cursor 热路径 monotonic_upsert SQL 用 per-call `format!` 重建 String（stress-verify 对抗审揪出）| SQL 提 `&'static const`；batch 路径按形状缓存留 ROADMAP P2 E9。scope_key:String 是运行时值非违反 |
