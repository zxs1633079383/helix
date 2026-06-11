---
id: HX-C004
title: Cargo features 必须 additive，禁止互斥 feature
status: active
severity: major
scope: helix
created: 2026-06-08
source: 2026-06-08 review + runtime-agnostic core 规约
recurrence_count: 1
trigger_paths:
  - "**/Cargo.toml"
trigger_keywords:
  - "[features]"
  - "mutually exclusive"
  - "disable"
  - "default-features"
  - "dep:"
related:
  - HX-C001
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
---

# HX-C004 — features 必须 additive

## §1 触发场景
- 改任何 crate 的 `[features]`
- 想加一个 "disable-X" / "no-X" / 平台二选一的 feature

## §2 背景（why）
Cargo **feature unification**：依赖图里同一 crate 的 features 只会被**求并集（永远只加不减）**。一旦设计了**互斥** feature（如 `native` vs `wasm` 二选一、`disable-storage`），在 diamond dependency 下两个都被打开 → 必爆，且报错极难定位。能力应是**可加的胶囊**：`eventbus`/`storage`/`http`/`ws` 各自独立、可任意子集组合、互不强依赖。

## §3 Required / Forbidden
```toml
# ✅ additive：每个能力一个 flag + optional dep + per-target deps
[features]
default  = ["eventbus"]
eventbus = []
storage  = ["dep:sqlx"]
http     = ["dep:reqwest"]
ws       = ["dep:tokio-tungstenite"]
[target.'cfg(target_arch="wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
```
```toml
# ❌ 互斥 feature（feature unification 会同时开 → 爆）
[features]
native = []
wasm   = []          # 与 native 互斥 = 反模式
disable-storage = [] # "关闭某能力" = 反模式，改成"默认不开 storage"
```
平台差异用 `[target.'cfg(...)'.dependencies]` + `#[cfg(target_arch)]` 表达，**不要用 feature 二选一**。

## §4 Verification
```bash
# 人工 + review：检查无 native/wasm 二选一、无 disable-* feature
grep -rnE '^(native|wasm|disable|no)[-_a-z]* *=' */Cargo.toml
# feature 矩阵抽样构建（任意子集都应编译）
cargo check -p helix-core --no-default-features
cargo check -p helix-core --no-default-features --features storage
cargo check -p helix-core --all-features
```

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | 骨架建立 | （预防性，来自 runtime-agnostic 规约 + review 确认）| feature unification 单调性 |

## §6 关联
- 上游：rules/runtime-agnostic-core.md §5
- 兄弟：HX-C001、HX-C003

## §7 历史与演进
- drafting → active：2026-06-08，workspace feature 约定确立。
