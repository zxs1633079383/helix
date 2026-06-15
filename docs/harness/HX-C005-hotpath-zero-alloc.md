---
id: HX-C005
title: 热路径零分配 O(1)（固定字符串用 &'static str 不 String；禁 read-modify-write）
status: active
severity: major
scope: helix
created: 2026-06-08
source: 2026-06-08 review（MAJ-5）+ 全局性能规约
recurrence_count: 2
trigger_paths:
  - "crates/helix-core/src/effect.rs"
  - "crates/helix-im/src/**"
  - "crates/helix-driver-*/src/storage.rs"
trigger_keywords:
  - ".to_string()"
  - "to_owned()"
  - "table: String"
  - "read-modify-write"
  - "UpsertSpec"
related:
  - HX-C007
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
---

# HX-C005 — 热路径零分配 O(1)

## §1 触发场景
- 写"每个事件至少命中一次"的路径：消息落库、cursor 推进、effect 构造、ack 计数
- 在 Effect / Spec 里放字符串字段
- 设计存储写操作

## §2 背景（why）
**MAJ-5（本仓 review 发现）**：热路径 `UpsertSpec.table: String`，每条消息都 `"messages".to_string()` 堆分配一次 —— 万人群刷屏时纯浪费。表名是**编译期常量**，应 `&'static str`。

通用铁律（全局性能规约）：每事件命中的写必须 **O(1)**（单 INSERT / UPDATE WHERE PK），**禁 read-modify-write**（读出 JSON 数组 → 应用层 push → 整列回写 = O(N²)）。这类性能坑在用户规模翻倍时才暴露，那时迁移代价指数级。

## §3 Required / Forbidden
```rust
// ✅ 编译期常量用 &'static str；热路径零堆分配
pub struct UpsertSpec { pub table: &'static str, pub row: Row }
const MESSAGES: &str = "messages";
// 单条 upsert = 1 INSERT ... ON CONFLICT DO UPDATE WHERE PK = O(1)
```
```rust
// ❌ 每条消息一次堆分配
pub struct UpsertSpec { pub table: String }
spec.table = "messages".to_string();          // 热路径 alloc

// ❌ read-modify-write 累计队列 = O(N²)
let mut arr: Vec<_> = read_json_column();
arr.push(x); write_json_column(arr);          // 改用 normalized 表 append=O(1)
```
累计/队列/多值用 normalized 表 + 复合主键，每元素一行，append=O(1) INSERT，清理=单 DELETE WHERE。

## §4 Verification
```bash
# 热路径结构体字段不应是 String（编译期常量场景）
grep -rn 'table: String\|: String,.*// hot' crates/helix-core/src/effect.rs
grep -rn '"[a-z_]*"\.to_string()' crates/helix-im/src crates/helix-driver-*/src/storage.rs
# review checklist：每个写 effect 复杂度 = O(1)？无 JSON 数组列 RMW？
```

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review MAJ-5 fix | `UpsertSpec.table: String` 每消息 `"messages".to_string()` 堆分配 | 热路径常量误用 owned String → 改 &'static str |
| 2026-06-15 | a0595607 review / c6a655a | `helix-driver-host` 把 cursor 热路径 `monotonic_upsert` SQL 用 per-call `format!` 重建 String（stress-verify 揪出）| 固定 SQL 模板误用 `format!` → 提 `&'static const`；batch 动态 SQL 形状缓存留 P2 |

## §6 关联
- 上游：~/.claude/rules/common/performance.md（热路径 O(1)）/ CLAUDE.md §2
- 兄弟：HX-C007（边界批处理）

## §7 历史与演进
- drafting → active：2026-06-08，MAJ-5 修复（&'static str）后确立。
