---
id: HX-C002
title: async port 的 Send 边界按三栖 cfg 切；跨 step/async 的状态禁用 &'a
status: active
severity: blocker
scope: helix
created: 2026-06-08
source: 2026-06-08 review（BLK-2 Tick<'a> blocker + Dioxus core 视角）
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/src/ports/**"
  - "crates/helix-core/src/engine.rs"
  - "crates/helix-core/src/tick.rs"
  - "crates/helix-im/src/state.rs"
trigger_keywords:
  - "Send + 'static"
  - "spawn_local"
  - "Sender<.*<'"
  - "async_trait"
  - "lifetime"
  - "Tick<'a>"
related:
  - HX-C001
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
---

# HX-C002 — async Send 边界 + 状态禁 `&'a`

## §1 触发场景
- 给 port 的 `async fn` 加/改 `Send` bound
- 在 core 的执行壳 / 模块状态里出现带生命周期参数的类型（`Foo<'a>`）放进 channel / async / 闭包
- 用 `async-trait` vs AFIT 取舍时

## §2 背景（why）
两个**实测 blocker**：
1. **BLK-2（本仓）**：`Tick<'a>`（借 `&'a [u8]`）满足不了 `Sender<Tick<'static>>`，timer 发送被注释掉、触发路径整条断。修复=载荷改 `bytes::Bytes`（owned）消除 `'a`。
2. **Send/!Send 三栖冲突**：native tokio multi-thread 要 `Send + 'static`；wasm 单线程 `spawn_local` 接 `!Send` future。port 的 async 签名若**无脑加 Send**，wasm 的 `!Send` future 冲突编不过；若**到处加生命周期**，进 async/'static 边界即地狱。

Dioxus 同源教训：0.5 弃 `Scope<'a>`（hook 返 `&'a T`，move 进 closure/async 时 lifetime 噩梦）→ Copy `Signal` + `GenerationalBox`（owned/Copy 句柄 + arena generation 校验）。状态默认 `UnsyncStorage`（!Send），跨线程才 opt-in `SyncStorage`。

## §3 Required / Forbidden
```rust
// ✅ 状态/载荷 owned，无生命周期，可随意进 async/channel/闭包
struct Tick { payload: bytes::Bytes }          // 不是 &'a [u8]
enum ModuleState { /* owned 字段 / Copy handle / arena Id */ }

// ✅ spawn 的 Send bound 按 target cfg 切（core 不硬加 Send）
#[cfg(not(target_arch="wasm32"))] fn spawn<F: Future<Output=()> + Send + 'static>(f: F);
#[cfg(target_arch="wasm32")]      fn spawn<F: Future<Output=()> + 'static>(f: F);
```
```rust
// ❌ 带生命周期的载荷塞进 'static 通道
struct Tick<'a> { payload: &'a [u8] }
let tx: Sender<Tick<'static>> = ...;           // 借用满足不了 → 路径断

// ❌ 在跨 wasm 的 port async 签名硬加 Send
trait Transport { async fn recv(&self) -> Msg where Self: Send; } // wasm !Send 冲突
```
**默认 Unsync（!Send）+ 跨线程才 opt-in Sync**；native 把 !Send 的 core 跑在专属单线程（`spawn_local`+`LocalSet`），并发归驱动。

## §4 Verification
```bash
# 找 core/im 里带生命周期的载荷/状态类型（放进 async 的应为 0）
grep -rnE 'struct .*<.*'\''[a-z]' crates/helix-core/src crates/helix-im/src
# 找硬加 Send 的 port async（按 cfg 切，不无脑加）
grep -rn 'Send + .static' crates/helix-core/src/ports
# 三栖编译都要绿
cargo check -p helix-core && cargo check -p helix-core --target wasm32-unknown-unknown
```

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review BLK-2 fix | timer send 被注释、触发路径断 | `Tick<'a>` 满足不了 `Sender<Tick<'static>>` → 改 bytes::Bytes |

## §6 关联
- 上游：rules/runtime-agnostic-core.md §3 §4
- 兄弟：HX-C001
- 对照：Dioxus Scope<'a>→Copy/GenerationalBox；UnsyncStorage 默认 / SyncStorage opt-in

## §7 历史与演进
- drafting → active：2026-06-08，BLK-2 修复（bytes::Bytes）后确立。
