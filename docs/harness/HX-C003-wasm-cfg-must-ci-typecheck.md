---
id: HX-C003
title: wasm-only 代码必须进 CI type-check；core 不许误链入 native binary
status: archived
severity: blocker
scope: helix
created: 2026-06-08
source: 2026-06-08 review（MAJ-2）
recurrence_count: 1
trigger_paths:
  - "crates/helix-driver-wasm/**"
  - "crates/helix-core/src/**"
  - ".github/workflows/**"
  - "**/Cargo.toml"
trigger_keywords:
  - "cfg(target_arch"
  - "wasm32-unknown-unknown"
  - "compile_error"
  - "CI target matrix"
related:
  - HX-C001
graduates_to:
  - AGENTS.md
---

# HX-C003 — wasm cfg 代码必须进 CI 三栖 type-check

> **归档状态（2026-06-15 / ADR-006）**：网页端/wasm 暂停维护，本文不再作为当前 CI 和交付验收要求。保留本卡是为了未来如果重启 Web 端，可以直接恢复 wasm target type-check 与误链守护。

## §1 触发场景
- 写 `#[cfg(target_arch="wasm32")]` 分支
- 新增/改 helix-driver-wasm
- 配置 CI

## §2 背景（why）
**MAJ-2（本仓 review 发现）**：`#[cfg(target_arch="wasm32")]` 的代码**只在编译该 target 时才被类型检查**。CI 若只跑默认 native `cargo check`，wasm 分支的类型错误**永远发现不了**，直到真编 wasm 才爆 —— 这是三栖项目的经典隐性坑（Dioxus/所有 cfg 跨端项目都踩过）。

配套：wasm driver（用 web-sys/OPFS）**不应被误链入 native binary**，否则把只在浏览器存在的符号拖进桌面端。

## §3 Required / Forbidden
```rust
// ✅ wasm-only crate 顶部加误链守护：非 wasm 编译直接报错
#[cfg(not(target_arch = "wasm32"))]
compile_error!("helix-driver-wasm 只能编译到 wasm32 target，不要链入 native binary");
```
```yaml
# ✅ CI 必须 per-target 各跑一次（两个分支都被 type-check）
- run: cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
- run: rustup target add wasm32-unknown-unknown
- run: cargo check -p helix-core -p helix-driver-wasm --target wasm32-unknown-unknown
```
```bash
# ❌ 只跑这一条 → wasm 分支永远不被检查
cargo check --workspace
```
> 注意：`cargo check --workspace`（不指定 target）对 helix-driver-wasm 会触发 `compile_error!` —— 这是**故意**的守护（MAJ-2 修复的一部分），不是 bug。workspace 级 check 应 `--exclude helix-driver-wasm`，wasm crate 单独按 target check。

## §4 Verification
```bash
grep -rn 'compile_error!' crates/helix-driver-wasm/src/lib.rs   # 期望: 命中守护
# CI 两条都必须存在且绿
cargo check --workspace --exclude helix-driver-wasm
cargo check -p helix-driver-wasm --target wasm32-unknown-unknown
```

## §5 Recurrence Log
| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review MAJ-2 fix | wasm 代码 native CI 完全跳过不被类型检查 | cfg 分支只在对应 target type-check；CI 缺 wasm target |

## §6 关联
- 上游：rules/runtime-agnostic-core.md §1
- 兄弟：HX-C001
- 对照：所有 cfg 跨端项目（Dioxus）通病

## §7 历史与演进
- drafting → active：2026-06-08，加 compile_error 守护 + CI wasm32 target 后确立。
- active → archived：2026-06-15，ADR-006 拍板网页端/wasm 暂停维护；当前产品路线只保留 PC Tauri + Flutter FFI。
