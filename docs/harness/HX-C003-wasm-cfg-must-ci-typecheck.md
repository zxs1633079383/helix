---
id: HX-C003
title: Web 双目标代码必须 native test + wasm final build；禁止 host runtime 泄漏
status: active
severity: blocker
scope: helix
created: 2026-06-08
source: 2026-06-08 review（MAJ-2）
recurrence_count: 2
last_recurred_at: "2026-07-14"
trigger_paths:
  - "crates/helix-driver-web/**"
  - "crates/helix-core/**"
  - "crates/helix-im/**"
  - ".gitlab-ci.yml"
  - "scripts/gate.sh"
  - "**/Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
trigger_keywords:
  - "cfg(target_arch"
  - "wasm32-unknown-unknown"
  - "wasm-bindgen"
  - "inventory"
  - "CI target matrix"
related:
  - HX-C001
graduates_to:
  - AGENTS.md
---

# HX-C003 — Web 双目标代码必须 native test + wasm final build

## §1 触发场景

- 修改 `helix-driver-web`、Web 所依赖的 core/im 边界或 target-specific 代码；
- 修改 Cargo feature、依赖、toolchain、CI 或 `scripts/gate.sh`；
- 注册表、constructor、`wasm-bindgen(start)` 等只有最终链接才会暴露问题的代码发生变化。

## §2 背景（why）

`#[cfg(target_arch = "wasm32")]` 只在对应 target 编译时接受类型检查；只跑 native 会让 Web 分支长期潜伏错误。
反过来，`helix-driver-web` 也有正式的 native 单测面，不能用只允许 wasm 编译的 `compile_error!` 封死。

A4 再次证明 `cargo check` 不够：`linkme` 在 native 编译正常，但 `wasm32-unknown-unknown` 最终制品链接时才暴露
不支持，最终迁移到 `inventory` 并由 Web bootstrap 初始化 constructors。因此正式门必须同时证明：

1. native 目标能编译并运行平台无关单测；
2. wasm final artifact 真正完成编译和链接；
3. 两个目标都没有把 host/native runtime 依赖拖进 Web。

WASM build 仍不是浏览器 E2E；hello、Fetch、send/echo、断线恢复必须由独立真实浏览器环境验证。

## §3 Required / Forbidden

```bash
# ✅ native/shared 类型面和单测面
cargo check -p helix-driver-web
cargo test -p helix-driver-web

# ✅ 必须 build，不能用 check 冒充 constructors/final-link 验证
cargo build --target wasm32-unknown-unknown -p helix-driver-web

# ✅ 两目标 strict clippy
cargo clippy -p helix-driver-web --all-targets --no-deps -- -D warnings
cargo clippy --target wasm32-unknown-unknown -p helix-driver-web --no-deps -- -D warnings
```

```rust
// ❌ helix-driver-web 的 native 单测是正式验收面，禁止沿用历史 wasm-only crate 的误链守护
#[cfg(not(target_arch = "wasm32"))]
compile_error!("wasm only");
```

依赖树禁止出现 `helix-driver-host/native/ffi/wasm`、`helix-host-cli`、Tokio、rusqlite、reqwest、sqlx、
Tauri、hyper、tungstenite、mio、socket2。`web-sys` 是 Web driver 的合法平台依赖，只在 `helix-core` 中禁止。

## §4 Verification

```bash
# 本地与 GitLab 共用同一事实源；包含 native test、wasm build、双目标 clippy 和依赖树守护
bash scripts/gate.sh

# runner 必须预装 target；air-gapped CI 不在运行时联网安装
rustup target list --installed | grep -qx wasm32-unknown-unknown
```

人工复核：CI 输出中的 wasm 步骤必须是 `cargo build`；浏览器 E2E 证据必须来自独立浏览器任务，不能把此卡绿色
写成“真浏览器/真 Go 已通过”。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-08 | review MAJ-2 fix | wasm 代码在 native CI 完全跳过 | cfg 分支只在对应 target type-check，CI 缺 wasm target |
| 2026-07-14 | A4 Web Rust platform driver | `linkme` native 正常，wasm final build 才失败 | 验收停在 native/type-check，没有证明最终制品 constructors 与链接 |

## §6 关联

- `scripts/gate.sh`：本地/CI 单一执行真源；
- HX-C001：core 零 runtime/platform 依赖；
- ADR-014：正式 Web driver 是 `helix-driver-web`，历史 `helix-driver-wasm` 不再作为交付对象。

## §7 历史与演进

- drafting → active：2026-06-08，建立 wasm target type-check；
- active → archived：2026-06-15，旧 Web 路线暂停；
- archived → active：2026-07-14，正式 `helix-driver-web` A4 落地，重写为 native + wasm final artifact 双目标守护。
