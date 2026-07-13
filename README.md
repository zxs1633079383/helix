# Helix — Runtime-agnostic 确定性业务内核基座

一次实现，两端运行：**PC 端 Tauri native** / **手机端 Flutter FFI**。
> 网页端（WASM）暂停维护——`helix-driver-wasm` 保留为历史实验代码，不进 roadmap / CI 必跑矩阵 / 交付验收（见 [`docs/harness/decisions.md`](docs/harness/decisions.md) ADR-006）。

## 架构简图

```
应用宿主（Tauri 桌面 / Flutter 手机端）
    │
    ▼
helix-driver-native ─┐
helix-driver-ffi    ─┴──► helix-core ◄── helix-im ◄── (helix-presence...)
                          （helix-driver-wasm = archived，见 ADR-006）
```

- **helix-core**：ports trait + Module trait + ExecutionShell（纯 sans-IO 状态机）
- **helix-im**：IM 业务模块，只依赖 helix-core，不含任何运行时 crate
- **helix-driver-***：各平台的 port 实现（tokio/sqlx 在此，不在 core）

详细架构见 [`docs/README.md`](docs/README.md)（文档地图）与 [`docs/过会方案/helix-core-im-架构设计.md`](docs/过会方案/helix-core-im-架构设计.md)（机制权威源）。

## 两端 Build 命令

### native（Tauri 桌面）

```bash
# 编译检查
cargo check -p helix-core -p helix-im -p helix-driver-native

# 运行 helix-core 单元测试（不依赖 tokio runtime）
cargo test -p helix-core

# 运行 helix-im 单元测试
cargo test -p helix-im
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
| `helix-driver-native` | tokio/sqlx/tungstenite/reqwest 实现（PC Tauri） | helix-core + 运行时 crate |
| `helix-driver-ffi` | C ABI/uniffi 实现（手机端 Flutter） | helix-core + 系统 crate |
| `helix-driver-host` | 共享 rusqlite Storage/network/event_sink（native+ffi 复用，ADR-007/010） | helix-core + 运行时 crate |
| `helix-driver-wasm` | ⚠️ **archived/experimental**（历史骨架，不作为正式 Web 壳） | — |

## 顶层目录地图

| 目录 | 作用 |
|---|---|
| `crates/` | 全部 Rust 源码，按「业务模块层 / core 层 / 平台驱动层」三层组织（见上「Crate 结构」） |
| `docs/` | 设计与工程文档：北极星 `GOAL.md`、`ROADMAP.md`、`COMMIT_CONVENTION.md`、`harness/`（踩坑卡 HX-C001~016 + 决策 ADR）、`过会方案/`（架构机制权威源）、`specs/` / `reports/` / `review/`、`agents/`（issue tracker / triage 约定） |
| `scripts/` | 工程验证与端到端脚本：`gate.sh` / `branch-sync-gate.sh` / `ddd-structure-gate.sh` + 一批 `host-cli-*-e2e.sh` 真链路用例 |
| `tools/` | 杂项工具（当前仅 `md2html.py`） |

## helix-im 内部业务模块

> IM 是跑在 core 上的第一个、当前唯一的业务模块；core 层本身不认识任何业务概念。

| 模块 / 目录 | 作用 |
|---|---|
| `module.rs` + `*_flow.rs` | IM 模块壳与编排：`send_flow`（发消息）/ `query_dispatch`（查询·对话渲染）/ `sync_flow`（同步）/ `lifecycle`（生命周期）/ `port_reply`（端口响应回灌） |
| `parser.rs` / `http_envelope.rs` | **防腐层（ACL 边界）**：WS 帧 / HTTP 响应 → 强类型 `InboundEvent`，边界零信任、零 `unwrap` |
| `acl/` | InboundEvent / Tick → `Effect` 翻译层，按职责裂分：transport / storage / sync_http / 投影 message / 投影 control effects |
| `channel.rs` + `channel_write/` | **聚合根 = channel gate**：封装 cursor 单调不回退不变量，三阶段 peek→persist→commit；`channel_write/` 落库 post / members / patch |
| `outbound/` | **出站命令族**（对齐现网写命令）：发帖 / 已读 / 频道增删改 / 成员管理 / 置顶 / 投票 / 紧急 / 模板已收到 / `bot_agent` 等，经 `registry.rs` 分发 |
| `sync_*.rs` | 增量同步引擎：`sync_session` / `sync_increment` / `sync_scheduler`——offline catch-up + online push 双轨，cursor 锚 nextSeq（HX-C008） |
| `ws/` | WS 运行时：`registry.rs` 策略注册表 + `handlers/` 各 action handler + `frame` / `context` |
| `render_ready*.rs` / `query*.rs` | 投影面：内核状态 → 前端可渲染 payload（消息 / 成员 / 回复 / 对话），经 EventSink emit `im:__bus__` |
| `state.rs` / `schema.rs` | IM 内存状态机 + SQLite schema（热路径 O(1) 写） |
| 其余 | `read_relay`（已读转发）/ `pending_send` + `send_reconcile`（乐观发送对账）/ `pong_compensate`（心跳补偿）/ `older_context`（历史回溯）/ `todo`（待办）/ `correlation`（corr_key 逐跳探针，HX-C015 可观测性） |

## 关键设计决策（摘要）

1. **helix-core 零运行时依赖**：`tokio::spawn` 在 driver，不在 core（审查意见1 P0）
2. **port trait 无关联类型**：统一返回 `Result<_, PortError>`，dyn-safe（审查意见2 P2）
3. **PortReply/Timer 定向路由**：`corr_map` 保证多模块并存时不误吞（审查意见2 P3）
4. **Module trait 4 方法**：删除 `tick()`，周期心跳用 `ScheduleTimer` self-arm（审查意见2 S1）
5. **EffectSink 统一**：EffectBuffer/EffectBatch/EffectSink 三名字合一（审查意见2 S4）
6. **写成功才推 cursor**：Persist{corr} → PortReply{Ok} → PersistFire{monotonic_upsert}（决策#2）
