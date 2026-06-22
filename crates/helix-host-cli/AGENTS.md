# helix-host-cli 局部指令（就近优先，全局见根 CLAUDE.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 是什么

最小 native 宿主 bin（sync-v2 Phase A 落地）：把 `ExecutionShell` + `ImModule` + `NativeTransport` 接成端到端可跑闭环，真连 Go 验证收发/握手。host 装配层 = 选 driver、建 ports、分配 TransportId、跑 `run_engine_loop_with_transports`。

> **身份定位**：host-cli 是 **headless native 宿主**——未来 Tauri `src-tauri` 的同构孪生（无 UI）。三层架构里它和 Tauri 同属平台驱动层；唯一区别是最外层 I/O 边界：host-cli = **stdin/stdout（终端遥控器）**，Tauri = **IPC bridge（前端 invoke / `im:__bus__` emit）**。中间 `ExecutionShell + ImModule + ports` 完全一致。它不是一次性测试产物，也不是出货客户端，而是 native 端**唯一无需前端就能真连 Go 的 e2e 宿主** + M2 Tauri 接入的参照实现。身份头注入（cookieId/companyId/deviceId）现走 env，未来 Tauri 改从登录态注入——driver 注入代码不变。

## 模块布局（src/）

- `main.rs` —— 装配：ports + 身份头 + transport + shell.register(ImModule) + 跑泵。
- `host_env.rs` —— ambient 环境派生（DB 多租户路径 + 本机 deviceId）。
- `repl.rs` —— **stdin 命令遥控器**：每行 → `Tick::Command` → 复用 `tick_tx` 入同一条泵（与 `Tick::Inbound` 同路，串行，零并发进 core）。坏命令不崩泵；`quit`/EOF → fire shutdown。

## stdin 遥控器命令（敲 `help` 看全集）

| 命令 | 映射 | 触发回来的 WS 事件（观察验证 parity） |
|---|---|---|
| `send <channelId> <text>` | `im_send_message` | posted-echo · posts_update（flood→too_long）|
| `quit` / `exit` / EOF | host shutdown | — |

> Tier 1 续接 outbound 自驱动全集（read/edit/leave/create/schedule/cancel/reconnect），各映射一个 helix-im 命令 handler（解析 → `Effect::Http` POST 到 Go，endpoint 取自现网 full-map 真源）。配合真 cses-client 操作可观察验证全部 13 inbound action。

## 本 crate 约定

1. **host 是 ambient API 合法落点**：`std::env`/`println`/`SystemTime` 等在此 allow（clippy `disallowed_methods=allow`）——与 driver 同级，core/im 才 deny。
2. **TransportId 由 host 集中分配**：`0 = 主 WS 连接`；第二模块/多端口（如 3399）用 1、2…，host 是唯一知道全部连接的地方（设计见 `docs/过会方案/helix-core-im-架构设计.md §4.4`）。
3. **身份头分层**：`cookieId=userId` 由 host 注入——**WS 握手头**（`with_handshake_headers`，Go 升级阶段建 session）**与** HTTP 全局头（`set_global_header`）两处都要带；`connectionId` 由 helix-im 收 hello 后盖章（core/im 不认识 userId）。
4. **connect 触发**：A1/A2 选 host 主动 `connect()`（连不上不退，进泵走 Send-warn 兜底）。`Effect::Connect` 意图驱动是后续可选项（未引入）。
5. 配置走 env：`HELIX_WS_URL` / `HELIX_API_BASE` / `HELIX_COOKIE_ID` / `HELIX_COMPANY_ID` / `HELIX_DB`；联调抓帧 `HELIX_DUMP_FRAMES=1`。

## 改动验收

```bash
cargo check -p helix-host-cli
# 真连 Go 冒烟（需起 Go 服务端 + 真 userId）
HELIX_COOKIE_ID=<userId> cargo run -p helix-host-cli   # 看 hello handshake complete + im:connection:established
```
