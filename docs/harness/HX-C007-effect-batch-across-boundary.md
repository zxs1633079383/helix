---
id: HX-C007
title: Effect/事件跨 Web/FFI 边界必须批处理，禁逐条过边界
status: active
severity: major
scope: helix
created: 2026-06-08
source: 2026-06-08 review（Dioxus core 视角，sledgehammer 对照）
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/src/engine.rs"
  - "crates/helix-driver-web/src/api.rs"
  - "crates/helix-driver-web/src/event_sink.rs"
  - "crates/helix-driver-web/browser/**"
  - "crates/helix-driver-ffi/src/**"
trigger_keywords:
  - "Vec<Effect>"
  - "flush_step"
  - "take_event_batches"
  - "on_domain_events_batch"
  - "extern"
  - "boundary"
related:
  - HX-C005
graduates_to:
  - AGENTS.md
---

# HX-C007 — Effect/事件跨 Web/FFI 边界批处理

## §1 触发场景

- core 一个 step 产生多条 Effect，driver 决定如何兑现并把结果交给 Web/FFI 宿主；
- Web 把一个 step 的 DomainEvent 交给 JS，或把待执行平台动作交给浏览器；
- FFI 把一个 step 的 DomainEvent 回调给 C++/Flutter 宿主。

## §2 背景（why）

跨 WASM/JS 或 FFI/C++ 边界每次都有固定开销。若每条 Effect/Event 单独 `invoke`、
`extern` 回调或 JS 调用，高频消息会把固定成本按条数放大。

Helix 的正式边界不是让 JS/C++ 逐条解释业务 Effect：Rust driver 先在进程内兑现 Effect，
再把一个 step 的事件形成批次；Web 一次 drain 待取批次，FFI 一次回调批 buffer。物理 WS
平台动作也通过有界数组 drain，不增加每个业务类型的 ABI。

## §3 Required / Forbidden

```rust
// ✅ Web：一个 step 先形成 EventBatch，再由一次 take_event_batches 调用批量取走。
event_sink.flush_step();
let batches = client.take_event_batches();

// ✅ FFI：一个 step 的事件编码成单个 buffer，一次回调宿主。
batch_sink.flush(events);
host.on_domain_events_batch(buffer);
```

```rust
// ❌ 每个事件单独穿越 JS/FFI 边界。
for event in events {
    js_or_ffi_callback(event);
}
```

- Web：`flush_step()` 冻结 step 批次；`take_event_batches()` 一次返回全部待取批次；
- FFI：每个 step 至多调用一次 `on_domain_events_batch`，wire 内部可容纳多条事件；
- JS/C++ 不获得逐业务 Effect ABI，不在边界外重新解释 Rust 业务流程。

## §4 Verification

```bash
rg -n 'flush_step|take_event_batches' \
  crates/helix-driver-web/src/api.rs crates/helix-driver-web/src/event_sink.rs
rg -n 'on_domain_events_batch|flush' crates/helix-driver-ffi/src
node --test crates/helix-driver-web/browser/helix-browser-adapter.test.mjs
bash scripts/ffi-verify.sh --no-demo
```

Review 必须确认：Web 的公开 drain 返回数组/批次，FFI 的宿主回调接收批 buffer；不能只靠
`grep for effect` 判断，因为 Rust driver 在边界内循环兑现 Effect 是合法实现。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review（Dioxus 视角）| （预防性）确立批处理契约，对照 Dioxus sledgehammer | 跨边界逐条穿越的固定开销线性叠加 |

## §6 关联

- 上游：`AGENTS.md` §2 Effect 跨 FFI 边界批处理；
- 兄弟：HX-C005（热路径零分配）；
- 对照：Dioxus interpreter 的批 buffer 思路。

## §7 历史与演进

- drafting → active：2026-06-08，Dioxus 视角 review 确立批处理契约；
- 2026-07-14：历史 wasm crate 删除后，守护对象迁到正式 `helix-driver-web` + FFI。
