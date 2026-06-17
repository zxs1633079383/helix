# helix-host-cli 局部指令（就近优先，全局见根 CLAUDE.md）

## 本 crate 是什么

最小 native 宿主 bin（sync-v2 Phase A 落地）：把 `ExecutionShell` + `ImModule` + `NativeTransport` 接成端到端可跑闭环，真连 Go 验证收发/握手。host 装配层 = 选 driver、建 ports、分配 TransportId、跑 `run_engine_loop_with_transports`。

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
