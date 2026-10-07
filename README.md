# Helix — Runtime-agnostic 确定性业务内核基座

一次实现，三端目标：**PC 端 Tauri native** / **手机端 Flutter FFI** / **Web WASM**。
> 当前交付事实：native/FFI 已产品化；Web 的 `helix-driver-web` Rust 平台壳、最小浏览器 glue、正式 gate 与真浏览器→真 Go 的鉴权、send/echo、断连恢复链已落地。Web crate 仍不包含产品 renderer，浏览器媒体上传也未实现。历史 `helix-driver-wasm` 已删除（见 [`docs/harness/decisions.md`](docs/harness/decisions.md) ADR-006/ADR-014）。

## 架构简图

```
应用宿主（Tauri 桌面 / Flutter 手机端 / Web）
    │
    ▼
helix-driver-native ─┐
helix-driver-ffi    ─┼──► helix-core ◄── helix-im
helix-driver-web    ─┘
```

- **helix-core**：ports trait + Module trait + ExecutionShell（纯 sans-IO 状态机）
- **helix-im**：IM 业务模块，依赖 helix-core 与纯逻辑支持库，不含平台运行时 crate
- **helix-driver-***：各平台的 port 实现（Tokio/rusqlite/reqwest/web-sys 等只在边缘）

当前代码与文档阅读从 [`docs/README.md`](docs/README.md) 开始；目标不变量见 [`docs/GOAL.md`](docs/GOAL.md)，实施状态见 [`docs/ROADMAP.md`](docs/ROADMAP.md)，架构决策见 [`docs/harness/decisions.md`](docs/harness/decisions.md)。

## 三端 Build 命令

### native（Tauri 桌面）

```bash
# 编译检查
cargo check -p cses-helix-core -p helix-im -p helix-driver-native

# 运行 helix-core 单元测试（不依赖 tokio runtime）
cargo test -p cses-helix-core

# 运行 helix-im 单元测试
cargo test -p helix-im
```

### FFI（iOS / Android）

```bash
# iOS (aarch64-apple-ios)
rustup target add aarch64-apple-ios
cargo check -p cses-helix-core --target aarch64-apple-ios
cargo check -p helix-driver-ffi --target aarch64-apple-ios

# Android (aarch64-linux-android，需要 Android NDK）
cargo check -p cses-helix-core --target aarch64-linux-android
```

### Web（A4 Rust 平台壳）

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown -p helix-driver-web
```

这条构建属于人工运行 `scripts/gate.sh` 的 HX-C003 检查项；当前 GitLab CI 的业务 job 只执行
MessageV3 real-chain（要求 Rust/Cargo 与 Node.js 20+），不执行 WASM、fmt 或 clippy。设置 `MESSAGEV3_DIAGNOSE=1` 时改走限时前置诊断与单 Gate `MV3-G07B`，不运行完整 83 Gate；诊断和日志说明见 [`scripts/README.md`](scripts/README.md#messagev3-ci-诊断与限时)。WASM 构建仍不能冒充浏览器 bootstrap 或真 Go E2E，不能据此宣称 Web 产品已经交付。

## 人工完整验收（`scripts/gate.sh`）

```bash
# helix-core 不含业务标识符（注释不计入）
HITS=$( (grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ || true) \
  | (grep -v '//' || true) | wc -l | tr -d ' ' )
[ "$HITS" -eq 0 ] && echo "PASS" || { echo "FAIL ($HITS)"; exit 1; }

# helix-core 依赖树不含运行时
if cargo tree -p cses-helix-core | grep -qiE 'tokio|sqlx|reqwest|web-sys|tauri'; then
  echo "FAIL (runtime dependency)"; exit 1
else
  echo "PASS"
fi
```

## Crate 结构

| Crate | 角色 | 依赖 |
|---|---|---|
| `cses-helix-core`（Rust crate `helix_core`） | ports + Module trait + ExecutionShell | serde / thiserror / bytes / async-trait |
| `helix-im` | MessageV3 IM 业务模块（无平台 runtime） | helix-core + serde/bytes 等纯逻辑依赖 |
| `helix-driver-native` | Tokio Native 执行与 PC/Tauri 接缝 | helix-core + helix-driver-host |
| `helix-driver-ffi` | 直接 C ABI、current-thread runtime 与批量事件接口 | helix-core + helix-im + helix-driver-host |
| `helix-driver-host` | 共享 rusqlite Storage/network/event_sink（native+ffi 复用，ADR-007/010） | helix-core + 运行时 crate |
| `helix-driver-web` | A4 Web Rust 平台壳：Browser Fetch、page-session store、外部 WS bridge、timer、有界 event batch v2 | helix-core + helix-im + 浏览器平台 crate |
| `helix-host-cli` | 本地 Go 联调、存储检查与真实链辅助入口 | helix-core + helix-im + helix-driver-host |
| `helix-perf` | workspace 内性能验证工具，不发布 crates.io | helix-core + helix-im + helix-driver-host |

## 顶层目录地图

| 目录 | 作用 |
|---|---|
| `crates/` | 全部 Rust 源码，按「业务模块层 / core 层 / 平台驱动层」三层组织（见上「Crate 结构」） |
| `docs/` | 设计与工程文档：北极星 `GOAL.md`、当前活账 `ROADMAP.md`、提交规范、`harness/`（HX-C001~018 + ADR）、`reports/` / `review/`、`agents/` |
| `specs/` | Spec Kit 需求、计划、任务、机器合同与分层验证记录；当前 MessageV3 真源为 Spec 047 |
| `scripts/` | 工程验证与端到端脚本：人工 `gate-fast.sh` / `gate.sh`、CI `messagev3/phase1-5-real-chain.mjs`、`branch-sync-gate.sh`、`ddd-structure-gate.sh` + 一批 `host-cli-*-e2e.sh` 真链路用例 |
| `tools/` | `helix-perf` 性能工具与 `md2html.py` 文档转换脚本 |

## helix-im 内部业务模块

> IM 是跑在 core 上的第一个、当前唯一的业务模块；core 层本身不认识任何业务概念。

| 模块 / 目录 | 作用 |
|---|---|
| `client_api/` | **三端业务调用门面**：公开命令名 + payload → canonical `AppCommand`；alias、归一化和 capabilities 的唯一真源，driver 只做平台错误映射 |
| `module.rs` + `module/` | IM 模块壳、配置、`Module` 实现与命令主分派；`send/`、`query/`、`sync/`、`channel/` 承担各自流程 |
| `ws/parser/` / `http_envelope.rs` | **防腐层（ACL 边界）**：WS 帧 / HTTP 响应 → 强类型 `InboundEvent`，边界零信任、零 `unwrap` |
| `acl/` | InboundEvent / Tick → `Effect` 翻译层，按职责裂分：transport / storage / sync_http / 投影 message / 投影 control effects |
| `channel/` + `channel/write/` | **聚合根 = channel gate**：封装 cursor 单调不回退不变量与频道生命周期；`write/` 落库 post / members / patch |
| `outbound/` | **出站命令族**（对齐现网写命令）：发帖 / 已读 / 频道增删改 / 成员管理 / 置顶 / 投票 / 紧急 / 模板已收到 / `bot_agent` 等，经 `registry.rs` 分发 |
| `sync/` | 增量同步引擎：`session` / `increment` / `scheduler`——offline catch-up + online push 双轨，cursor 锚 nextSeq（HX-C008） |
| `ws/` | WS 运行时：`registry.rs` 策略注册表 + `handlers/` 各 action handler + `frame` / `context` |
| `event/` / `query/` | MessageV3 事件构造与 local-first 查询、时间线读回；通过 EventSink 交付 `{event,data}` |
| `state.rs` + `state/` / `schema.rs` | IM 内存状态机、强类型值与 SQLite schema（热路径 O(1) 写） |
| 其余 | `query/read_relay`（已读转发）/ `send/pending` + `send/reconcile`（乐观发送对账）/ `send/pong_compensate`（心跳补偿）/ `channel/older_context`（历史回溯）/ `draft` / `todo` / `chain` / `correlation` |

## 关键设计决策（摘要）

1. **helix-core 零运行时依赖**：`tokio::spawn` 在 driver，不在 core（审查意见1 P0）
2. **port trait 无关联类型**：Storage、HTTP、FrameSender 等 I/O 端口使用 `PortError` 表达失败；Clock/Random/EventSink 等端口按各自方法签名返回（见 [`ports/`](crates/helix-core/src/ports/)）
3. **PortReply/Timer 定向路由**：`corr_map` 保证多模块并存时不误吞（审查意见2 P3）
4. **Module trait 5 方法**：`name`、`accepts`、`handle`、`on_start`、`on_stop`；后两者有默认实现，周期心跳用 `ScheduleTimer` self-arm（见 [`module_host.rs`](crates/helix-core/src/module_host.rs)）
5. **EffectSink 统一**：EffectBuffer/EffectBatch/EffectSink 三名字合一（审查意见2 S4）
6. **写成功才推 cursor**：Persist{corr} → PortReply{Ok} → PersistFire{monotonic_upsert}（决策#2）
