# Helix 全仓架构索引

> 快照基线：本地 `main@e0252dc58830eabd2653e28c6d3277995595775a`（2026-07-13）。
> 本目录按当前代码生成，代码与本文冲突时以代码、`docs/GOAL.md`、`docs/harness/decisions.md` 的顺序裁决。

## 1. 这套文档解决什么

这不是历史设计稿的重新排版，而是当前 `main` 的代码阅读地图。它同时回答四类问题：

1. Helix 为什么分成 core、业务模块、共享 host driver 和平台薄壳。
2. 一个 `Tick` 如何进入纯状态机，变成 `Effect`，再由宿主兑现为 SQLite、HTTP、WebSocket、上传、定时器和领域事件。
3. IM 的命令、发送、回声对账、增量同步、离线同步、会话写入、本地查询与 render-ready 投影分别落在哪些目录。
4. native、FFI、host-cli、归档 wasm 以及 scripts、harness、trace 如何围绕同一内核工作。

所有流程图均为 Mermaid MMD 语法；状态名、状态迁移和流程边标签使用中文。Rust 没有传统 OOP 类，本文的“类图”统一映射为 `struct`、`enum`、`trait`、类型别名及其实现/组合关系。

## 2. 覆盖边界

“全部文件夹”按架构意义解释，避免把 `.git`、IDE 缓存、编译产物和历史报告误当运行时模块：

- 完整覆盖 7 个 workspace crate。
- 完整覆盖每个 `crates/*/src` 根目录和全部源码子目录。
- 覆盖各 crate 的 `tests/`、`benches/`、`contracts/`、`docs/design/`、FFI `smoke/` 等验证边界。
- 覆盖根目录 `crates/`、`docs/`、`scripts/`、`tools/`、`docker/otel/`、`.agents/` 的工程支撑流。
- 排除 `.git/`、`.idea/`、`target/`、`worktrees/`、`smoke/build/` 等非源码或生成目录。

逐目录对账见 [目录覆盖清单](./10-目录覆盖清单.md)。

## 3. 推荐阅读顺序

### 第一遍：建立心智模型

| 顺序 | 文档 | 目标 |
|---:|---|---|
| 1 | [`docs/GOAL.md`](../GOAL.md) | 先记住 runtime-agnostic、sans-IO、确定性和双端边界 |
| 2 | [系统总览](./00-系统总览.md) | 看清依赖方向与端到端主链 |
| 3 | [helix-core](./01-helix-core.md) | 掌握 `Tick -> Module -> Effect` 语言 |
| 4 | [helix-im](./02-helix-im.md) | 掌握业务状态机与目录分工 |
| 5 | [helix-driver-host](./03-helix-driver-host.md) | 掌握 Effect 兑现、回灌与背压 |
| 6 | [native 与 host-cli](./04-native与host-cli.md) | 掌握 PC/联调装配入口 |
| 7 | [FFI](./05-helix-driver-ffi.md) | 掌握手机端 C ABI、批回调和生命周期 |
| 8 | [归档 wasm](./06-helix-driver-wasm-归档.md) | 知道历史边界，不把它当当前交付项 |
| 9 | [工程支撑](./07-工程支撑与验证.md) | 掌握 gate、trace、strict matrix、agent 治理 |

### 第二遍：按业务链路阅读

| 顺序 | 业务链 | 阅读入口 |
|---:|---|---|
| 1 | 应用命令进入内核 | `helix-core::Tick::Command` -> `ImModule::handle` |
| 2 | 普通消息发送 | `helix-im/src/send/` -> `outbound/send_build.rs` -> host HTTP |
| 3 | WS 回声对账 | `helix-im/src/ws/handlers/post.rs` -> `send/reconcile.rs` |
| 4 | 增量同步 | `helix-im/src/sync/` -> `ws/handlers/increment_channel*.rs` |
| 5 | 离线同步 | `sync/session.rs` -> ACL HTTP/Persist Effect -> `PortReply` |
| 6 | 会话与成员落库 | `channel/` -> `channel/write/` -> ACL storage effects |
| 7 | 本地查询与投影 | `query/` -> `query/render_ready/` -> `im:__bus__` |
| 8 | Effect 兑现 | `helix-driver-host/src/engine.rs` -> `dispatch.rs` |
| 9 | 平台出口 | native broadcast 或 FFI `on_domain_events_batch` |

### 第三遍：按风险边界复核

1. `docs/harness/HX-C001..C007`：core 纯度、Send、feature、热路径、FFI 和批处理。
2. `HX-C008`、`HX-C010`：同步 cursor 与有序 Effect。
3. `HX-C016`：真 Go e2e 环境与 host-cli 单连接规则。
4. `docs/trace/trace-scope-contract.md`：逐跳 correlation 与证据边界。
5. `crates/helix-im/docs/Strict Matrix/README.md`：UC 发布证据，不把文档存在误判成 runtime green。

## 4. 模块索引

| 模块 | 当前角色 | 主要入口 | 详细文档 |
|---|---|---|---|
| `helix-core` | 运行时无关执行壳、模块宿主、通用 ports/Effect | `ExecutionShell::step` | [01](./01-helix-core.md) |
| `helix-im` | IM 领域状态机、命令、WS、同步、查询投影 | `ImModule::handle` | [02](./02-helix-im.md) |
| `helix-driver-host` | native/FFI 共用的泵、SQLite、HTTP、WS、上传、trace | `run_engine_loop` | [03](./03-helix-driver-host.md) |
| `helix-driver-native` | PC/Tauri 具体端口与事件广播薄壳 | `run_engine_loop_with_transports` | [04](./04-native与host-cli.md) |
| `helix-host-cli` | 真 Go 联调和本地 store 检查入口 | `main` / `store_inspect_main` | [04](./04-native与host-cli.md) |
| `helix-driver-ffi` | C ABI、专用 pump 线程、批量事件回调 | `helix_create_with_network` | [05](./05-helix-driver-ffi.md) |
| `helix-driver-wasm` | 历史实验，ADR-006 后归档 | `run_engine_loop` stub | [06](./06-helix-driver-wasm-归档.md) |
| 工程支撑 | gate、回放、e2e、trace、OTel、agent 路由 | `scripts/gate.sh` 等 | [07](./07-工程支撑与验证.md) |

## 5. 真源与防漂移

- 架构不变量：[`GOAL.md`](../GOAL.md)。
- ADR：[`harness/decisions.md`](../harness/decisions.md)。
- 当前进度：[`ROADMAP.md`](../ROADMAP.md)，但 green 必须再看当轮 gate/collector 证据。
- IM 命令真源：`crates/helix-im/src/module.rs`、`outbound/registry.rs`、`mobile_contract.rs`。
- WS 真源：`crates/helix-im/src/ws/registry.rs` 与 `ws/handlers/`。
- 投影 schema 真源：`crates/helix-im/docs/projection-schema.md` 与 `tests/projection_schema_test.rs`。
- 本目录只描述 `main@e0252dc`。后续新增/移动源码目录时，必须同步 [目录覆盖清单](./10-目录覆盖清单.md) 并重新运行 Mermaid 校验。
