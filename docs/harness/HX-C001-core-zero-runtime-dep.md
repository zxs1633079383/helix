---
id: HX-C001
title: helix-core 永不依赖任何运行时/平台 crate（只定义 ports）
status: active
severity: blocker
scope: helix
created: 2026-06-08
source: 2026-06-08 多专家 review（runtime-agnostic core 视角）+ 产品定位铁律
recurrence_count: 1
trigger_paths:
  - "crates/helix-core/**"
trigger_keywords:
  - "use tokio"
  - "use sqlx"
  - "use reqwest"
  - "web_sys"
  - "use tauri"
  - "helix-core Cargo.toml dependencies"
related:
  - HX-C002
  - HX-C006
graduates_to:
  - ~/.claude/rules/rust/runtime-agnostic-core.md
---

# HX-C001 — helix-core 零运行时 / 零平台依赖

## §1 触发场景
- 改 `crates/helix-core/Cargo.toml` 的 `[dependencies]`
- 在 helix-core 里 `use` 任何具体运行时/平台 crate
- 想"图方便"在 core 直接调 tokio::spawn / sqlx::query / reqwest::get / web_sys::* / tauri::*

## §2 背景（why）
Helix 的全部价值在于 **core 一次实现，当前产品化复用到 PC Tauri/native 与 Flutter FFI 两端**。
wasm/web 已按 ADR-006 归档为历史实验，不进入当前交付验收；但 core 仍不得被任何具体运行时污染，
否则未来端扩展和现有双端复用都会碎。core 一旦 import 任一具体运行时：
- `tokio`（multi-thread）把 core 绑到异步 runtime 假设上，破坏 FFI/单线程宿主边界
- `sqlx` 拉 tokio/mio/getrandom → wasm 碎
- `web_sys` 只在 wasm 有 → native 碎
- `tauri` 把 core 绑死单一宿主 → ffi/wasm/移动全废

这是产品的**第一性铁律**，不是优化项。Dioxus 同款：dioxus-core 是 headless 无平台 crate，renderer 才依赖具体平台 —— 正是它能上 6 端的根因。

## §3 Required / Forbidden
```rust
// ✅ helix-core：只定义 trait（port），让驱动实现
pub trait Storage { async fn upsert(&self, spec: UpsertSpec) -> Result<(), PortError>; }
pub trait Clock   { fn now_ms(&self) -> u64; }
```
```rust
// ❌ helix-core 里直接用具体运行时
use tokio::time::sleep;          // 编不过 wasm
use sqlx::SqlitePool;            // 拉 tokio，wasm 碎
use web_sys::WebSocket;          // native 碎
tokio::spawn(async { ... });     // core 不许持有运行时
```
core 只能依赖：`serde` / `thiserror` / `futures-channel` / `bytes` 等**运行时中立**的纯库。

## §4 Verification
```bash
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri|mio'   # 期望: 空
grep -rnE '^\s*use (tokio|sqlx|reqwest|web_sys|tauri)' crates/helix-core/src # 期望: 0
# 当前必跑产品目标：PC Tauri/native + Flutter FFI
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi

# 可选历史诊断：wasm/web 已归档，不进入当前交付验收或 CI 必跑矩阵
cargo check -p helix-core --target wasm32-unknown-unknown
```
CI gate：上述 `cargo tree | grep` 非空即 fail。

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review 合成阶段 | 初版设计曾把 tokio/sqlx/tungstenite bake 进"im_core" | 单平台假设；本轮 review 拆出三层后锁定本不变量 |

## §6 关联
- 上游：CLAUDE.md §0 依赖铁律 / rules/runtime-agnostic-core.md §2
- 兄弟：HX-C002（Send 边界）、HX-C006（FFI）
- 对照：Dioxus dioxus-core headless（不依赖 dioxus-signals/平台）

## §7 历史与演进
- drafting → active：2026-06-08，骨架三栖编译验证 helix-core 仅依赖 serde/thiserror/futures-channel 时确立。
