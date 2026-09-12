# helix-host-cli — 设计图索引

> native 最小宿主二进制：把 `ExecutionShell` + `ImModule` + `helix-driver-native` 接成端到端可跑闭环，用于**真连现网 Go 服务端**联调（L2 冒烟）。默认连接 `localhost:8066`；历史 8065 实测只保留为旧运行态证据。

| 图 | 一句话 |
|---|---|
| [assembly-and-connect.mmd](./assembly-and-connect.mmd) | ✅ 装配链路：建 ExecutionShell→register(ImModule)→注入身份头→connect 主 WS→进 engine_loop |
| [connect-handshake.mmd](./connect-handshake.mmd) | ✅ 真 Go WS 握手时序（cookieId 有效→hello+increment；空→~5s 超时关，优雅断连，实测） |

## 角色
- **不是产品交付端**（产品端是 native Tauri 壳 + Flutter FFI 壳），是**联调/冒烟宿主**。
- 配置真源：`helix.config.yaml`（`--config` 可覆盖路径）；默认 `ws://127.0.0.1:8066/api/v4/websocket` 与 `http://127.0.0.1:8066/api/cses`。`--cookie-id`/`--company-id` 只覆盖联调身份，`HELIX_*` 逃生口仅 debug 构建有效，release 不作为配置合同。
- 现网 Go 在 WS 升级阶段按 `cookieId=userId` 建 session；session 空则不发 hello、~5s 超时关（实测验证）。

## Trace 环境配置

`observability.otel.attributes["deployment.environment"]` 已由
[`Otel::host_config`](../../src/config_observability.rs) 传入 `HostOtelConfig`；缺省为 `local`。
Host exporter 将值去空白、转小写，并限定为 `local/test/dev/pre/prod`，未知值回退 `local`。
这只说明该环境字段已接线，不表示 `attributes` 中任意键都会被导出；
Metrics 的环境仍使用独立的 `observability.metrics.deploymentEnvironment`。
真源与回归见 [`otel.rs`](../../../helix-driver-host/src/otel.rs)、
[`exporter.rs`](../../../helix-driver-host/src/otel/exporter.rs) 和
[`config_tests.rs`](../../src/config_tests.rs)。本次仅静态对账，不代表 Collector 或 PRE 已采集。

## 关联
- 全景：[`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) （当前 P0-P7 活账与证据边界）
- engine 泵真源：[`helix-driver-native/docs/design`](../../../helix-driver-native/docs/design/)
