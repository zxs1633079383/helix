# Helix Harness 跨卡复现日志

> 每次再次踩到任一 HX-C{NNN} 的坑 → 这里追一行 + 对应卡 §5 +1 + frontmatter recurrence_count +1（同 commit 三同步）。

| 日期 | 卡 | session/commit | 现象 | 处置 |
|---|---|---|---|---|
| 2026-06-08 | HX-C001..C007 | 多专家 review wf_281205c2 | 首批 7 条约束建立（3 blocker + 5 major review 发现 + Dioxus 对照 + runtime-agnostic 规约）| 卡片创建 |
| 2026-06-15 | HX-C005 | a0595607 review / commit c6a655a | a0595607 重构把 cursor 热路径 monotonic_upsert SQL 用 per-call `format!` 重建 String（stress-verify 对抗审揪出）| SQL 提 `&'static const`；batch 路径按形状缓存留 ROADMAP P2 E9。scope_key:String 是运行时值非违反 |
| 2026-06-16 | HX-C008 | B3 修复（本会话）| 139 个 `sync response missing 'type'`：parser 按废弃抽象 `{type:...}` 解，真 wire 是 `{status,data:{entries:[SyncEntry]}}`；用户 challenge 后核 cses-client + Go 实证定契约 | 新建 HX-C008；parser 切真 wire + cursor 锚 nextSeq + 续拉 len≥500 + 回归测试 |
| 2026-06-18 | HX-C010 | NS-1 修复 `2d50405`（北极星 review wf_69cf287f）| proactive-resync 从 std HashMap/HashSet 迭代序产有序 sync Effect + 单调 corr，SipHash 随机种子→同输入不同输出，破坏可回放（lifecycle_test 只断 len 掩盖乱序）| 新建 HX-C010；迭代前 `sort_unstable` 锚 ChannelId 升序 + 确定性回放测试；过回放闸门（不误伤既往 trap）|
