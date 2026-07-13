---
id: HX-C007
title: Effect 跨 wasm/FFI 边界必须批处理，禁逐条过边界
status: active
severity: major
scope: helix
created: 2026-06-08
source: 2026-06-08 review（Dioxus core 视角，sledgehammer 对照）
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/src/engine.rs"
  - "crates/helix-driver-wasm/**"
  - "crates/helix-driver-ffi/**"
trigger_keywords:
  - "Vec<Effect>"
  - "for effect in"
  - "invoke"
  - "postMessage"
  - "JsValue"
  - "boundary"
related:
  - HX-C005
graduates_to:
  - AGENTS.md
---

# HX-C007 — Effect 跨边界批处理

## §1 触发场景
- core 一个 step 产出 `Vec<Effect>` 后，driver 如何把它送过 wasm/FFI 边界
- 写 wasm driver 的 effect 解释器 / FFI driver 的 effect 回传

## §2 背景（why）
跨 wasm / FFI 边界**每次穿越都有固定开销**（序列化 + 上下文切换）。若 core 每产出一个 effect 就单独 `invoke` / `postMessage` / 过一次 JsValue 边界，高频场景（消息洪峰）开销随 effect 数线性叠加 → 性能坑。

Dioxus 的实战解法（sledgehammer）：把多条 mutation 编码成**单个 byte buffer**，**一次**跨 wasm 边界传 JS 端 decode 执行 —— 而非逐条 web-sys 调用。性能差异数量级。这正是 Helix 的 Effect 该走的路。

## §3 Required / Forbidden
```rust
// ✅ 一个 step 的 Vec<Effect> 整批交给 driver，一次过边界
let effects: Vec<Effect> = core.step(inputs, &mut state);
driver.apply_effects(&effects);          // 批量；driver 内部一次 postMessage/一次 buffer
```
```rust
// ❌ 逐条过边界
for effect in core.step(inputs, &mut state) {
    js_invoke(effect);                   // 每条一次 wasm 边界穿越 = N 次开销
}
```
- wasm：driver 把整批 effect 编码成一段 buffer（serde-wasm-bindgen / 自定义 byte protocol），一次交 JS。
- FFI：整批 effect 一次回调 host，不每条一次 extern 调用。

## §4 Verification
```bash
# 边界层不应在 for effect 循环里逐条过边界
grep -rn 'for .* in .*effect' crates/helix-driver-wasm/src crates/helix-driver-ffi/src
grep -rn 'invoke\|postMessage\|extern' crates/helix-driver-wasm/src crates/helix-driver-ffi/src
# review：确认 apply_effects 接收 &[Effect] 整批，不是单个 Effect
```

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review（Dioxus 视角）| （预防性）确立批处理契约，对照 Dioxus sledgehammer | 跨边界逐条穿越的固定开销线性叠加 |

## §6 关联
- 上游：rules/runtime-agnostic-core.md §8
- 兄弟：HX-C005（热路径零分配）
- 对照：Dioxus dioxus-interpreter-js sledgehammer byte protocol

## §7 历史与演进
- drafting → active：2026-06-08，Dioxus 视角 review 确立批处理契约。
