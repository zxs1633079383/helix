# Helix — Runtime-agnostic 确定性业务内核基座

一次实现，三栖运行：**Tauri native** / **FFI (iOS + Android)** / **WASM (browser)**。

## 架构简图

```
应用宿主（Tauri / Mobile Host / Browser JS）
    │
    ▼
helix-driver-native ─┐
helix-driver-wasm   ─┼──► helix-core ◄── helix-im ◄── (helix-presence...)
helix-driver-ffi    ─┘
```

- **helix-core**：ports trait + Module trait + ExecutionShell（纯 sans-IO 状态机）
- **helix-im**：IM 业务模块，只依赖 helix-core，不含任何运行时 crate
- **helix-driver-***：各平台的 port 实现（tokio/sqlx/web-sys 在此，不在 core）

详细架构见 [`docs/helix-two-layer-architecture.md`](docs/helix-two-layer-architecture.md)。

## 三栖 Build 命令

### native（Tauri 桌面）

```bash
# 编译检查
cargo check -p helix-core -p helix-im -p helix-driver-native

# 运行 helix-core 单元测试（不依赖 tokio runtime）
cargo test -p helix-core

# 运行 helix-im 单元测试
cargo test -p helix-im
```

### wasm32（浏览器）

```bash
# 编译检查（需要 wasm32-unknown-unknown target）
rustup target add wasm32-unknown-unknown
cargo check -p helix-core --target wasm32-unknown-unknown
cargo check -p helix-im --target wasm32-unknown-unknown

# ⚠️ helix-driver-wasm 的 OPFS 存储需要特殊编译标志：
# RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals" \
#   cargo check -p helix-driver-wasm --target wasm32-unknown-unknown

# wasm-pack 打包
# wasm-pack build crates/helix-driver-wasm --target bundler
```

### FFI（iOS / Android）

```bash
# iOS (aarch64-apple-ios)
rustup target add aarch64-apple-ios
cargo check -p helix-core --target aarch64-apple-ios
cargo check -p helix-driver-ffi --target aarch64-apple-ios

# Android (aarch64-linux-android，需要 Android NDK）
cargo check -p helix-core --target aarch64-linux-android
```

## 验收红线（CI 强制）

```bash
# helix-core 不含任何 IM 业务词汇
grep -r "channel\|message\|cursor\|temporaryId\|im:" \
     crates/helix-core/src/  && echo "FAIL" || echo "PASS"

# helix-core 不含 tokio/sqlx/web-sys
grep -E "tokio|sqlx|reqwest|tungstenite|web.sys" \
     crates/helix-core/Cargo.toml && echo "FAIL" || echo "PASS"
```

## Crate 结构

| Crate | 角色 | 依赖 |
|---|---|---|
| `helix-core` | ports + Module trait + ExecutionShell | serde / thiserror / bytes / async-trait |
| `helix-im` | IM 业务模块（BC-B） | helix-core 唯一 |
| `helix-driver-native` | tokio/sqlx/tungstenite/reqwest 实现 | helix-core + 运行时 crate |
| `helix-driver-wasm` | wasm-bindgen/web-sys/sqlite-wasm-rs 实现 | helix-core + wasm crate |
| `helix-driver-ffi` | C ABI/uniffi 实现 | helix-core + 系统 crate |

## 关键设计决策（摘要）

1. **helix-core 零运行时依赖**：`tokio::spawn` 在 driver，不在 core（审查意见1 P0）
2. **port trait 无关联类型**：统一返回 `Result<_, PortError>`，dyn-safe（审查意见2 P2）
3. **PortReply/Timer 定向路由**：`corr_map` 保证多模块并存时不误吞（审查意见2 P3）
4. **Module trait 4 方法**：删除 `tick()`，周期心跳用 `ScheduleTimer` self-arm（审查意见2 S1）
5. **EffectSink 统一**：EffectBuffer/EffectBatch/EffectSink 三名字合一（审查意见2 S4）
6. **写成功才推 cursor**：Persist{corr} → PortReply{Ok} → PersistFire{cursor_upsert}（决策#2）
