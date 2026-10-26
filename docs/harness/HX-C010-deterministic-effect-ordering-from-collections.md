---
id: HX-C010
title: 业务态机「集合迭代 → 有序 Effect 生产」必须确定性有序（用有序集合 / 迭代前 sort）
status: active
severity: major
scope: helix
created: 2026-06-18
source: 2026-06-18 会话——北极星符合性 review wf_69cf287f 抓 NS-1（proactive-resync 从 std HashMap/HashSet 迭代序产出有序 Effect + 单调 corr，破坏可回放）；过回放闸门（挡 NS-1 + 不误伤 log.md 历史 trap）后建卡
recurrence_count: 1
trigger_paths:
  - "crates/helix-im/src/module.rs"
  - "crates/helix-im/src/ws/handlers/increment_channel_end.rs"
trigger_keywords:
  - "HashMap"
  - "HashSet"
  - "keys()"
  - "iter()"
  - "alloc_corr"
  - "确定性"
  - "可回放"
  - "sort_unstable"
related:
  - HX-C005
  - HX-C009
graduates_to:
inline_target:
---

# HX-C010 — 集合迭代 → 有序 Effect 生产必须确定性有序

## §1 触发场景（Trigger）

在业务态机（helix-im 等业务模块 / core）里遍历 `HashMap` / `HashSet` 等**无序集合**，把迭代结果喂进**有序 Effect 生产路径**（逐元素单调 `alloc_corr` + `out.push(Effect)`，或顺序敏感的 HTTP/Persist/Send 序列）。grep:`channels.keys()`、`*_fetched.iter()`、`HashMap`/`HashSet` + 同段 `alloc_corr`/`out.push`。

## §2 背景（why）

2026-06-18 北极星符合性 review（wf_69cf287f）抓 NS-1:`emit_proactive_resync_for` 的 targets 来自 std `HashMap<ChannelId,_>::keys()`（module.rs）与 `HashSet<ChannelId>::iter()`（increment_channel_end.rs），而 std 默认 `RandomState`=SipHash **每进程随机种子** → 迭代序逐进程随机;该路径逐 channel **单调** `alloc_corr` + `push` 有序 sync_notify Http Effect → **同输入两次运行 / native vs FFI 吐出的 Effect 顺序与 corr↔channel 配对不一致**，破坏 GOAL §1「同输入同输出可回放」。

> 核心:确定性不只是 core 本体不调 `Instant::now`/`rand`——**业务态机内任何「无序集合 → 有序输出」的转换都是隐藏的不确定源**。SipHash 随机种子让它在单机单测里"碰巧有序"而潜伏,多端 / 重放才暴露。`lifecycle_test` 当时只断言 `syncs.len()==3` 没 pin 顺序,掩盖了乱序。

## §3 Required / Forbidden

```rust
// ✅ 迭代前对集合排序（ChannelId 已 derive Ord,冷路径零热代价）
let mut targets: Vec<ChannelId> = self.state.channels.keys().copied().collect();
targets.sort_unstable();
self.emit_proactive_resync_for(&targets, out);   // 逐元素 alloc_corr 现确定性

// ✅ 或换有序集合（与 channel.rs BTreeMap 既有纪律对齐）:
//    需迭代序的字段用 BTreeMap/BTreeSet,append=O(log n),迭代天然有序
```

```
❌ HashMap::keys() / HashSet::iter() 直接 collect 后喂 alloc_corr + 有序 push（迭代序随机 → corr 配对漂移）
❌ 靠单测「碰巧通过」放行——单测只断 len/集合相等、不 pin 顺序,掩盖乱序（潜伏 flaky）
❌ 以「order 无所谓」放行却没在注释证明该路径真 order-无关
```

## §4 Verification

- 回放测试 pin 顺序（防 NS-1 回归）:≥2 元素、非排序插入,断言输出 Effect 的元素顺序 == 确定性排序（见 `crates/helix-im/tests/lifecycle_test.rs::proactive_resync_emits_channels_in_deterministic_sorted_order`）。
- grep gate（人工 review lens）:`grep -nE '\.(keys|iter)\(\)' crates/*/src` 命中处人工核「是否喂有序 Effect 生产」——是则必须前置 `sort` 或换 `BTree`。
- 确定性回放:同 Tick 序列两次运行 Effect 序列逐字相等（core 已有 `request_reply_replay::replay_determinism_same_ticks_same_effects` 范式）。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-18 | NS-1 修复 `2d50405` | proactive-resync 从 HashMap/HashSet 迭代序产有序 sync Effect + 单调 corr,破坏可回放 | 无序集合 → 有序输出的转换未确定性排序;SipHash 随机种子使其潜伏 |

## §6 关联

- 上游:GOAL §1 确定性（time/rand/id host 注入 + 同输入同输出可回放）。
- 兄弟 harness:HX-C005（同为热路径/集合纪律——HX-C005 管「写复杂度 O(1) + 字面 `&'static`」,本卡管「集合迭代序的确定性」,正交互补）;HX-C009（同为「确定性闭环的隐藏进料口」——HX-C009 管预期对账进料口,本卡管 Effect 生产进料口）。
- 回放闸门:本卡过 `docs/harness/log.md`（HX-C005/C008/C001-7 三 trap 均无关维度,不误伤）+ NS-1 命中后建卡（防约束过拟合/膨胀）。

## §7 历史与演进

- drafting → active:2026-06-18 第一次命中（NS-1,wf_69cf287f）+ 过回放闸门。
- 待办:第二个业务模块（presence/notify）接入时复核——多模块 Effect 交织时,跨模块有序性是否需进一步约束（当前单模块内排序已足）。
