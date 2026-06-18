# helix-host-cli — 设计图索引

> native 最小宿主二进制：把 `ExecutionShell` + `ImModule` + `helix-driver-native` 接成端到端可跑闭环，用于**真连现网 Go 服务端**联调（L2 冒烟）。已实测真连 `localhost:8065` WS 升级成功 + 握手 + increment + 落库。

| 图 | 一句话 |
|---|---|
| [assembly-and-connect.mmd](./assembly-and-connect.mmd) | ✅ 装配链路：建 ExecutionShell→register(ImModule)→注入身份头→connect 主 WS→进 engine_loop |
| [connect-handshake.mmd](./connect-handshake.mmd) | ✅ 真 Go WS 握手时序（cookieId 有效→hello+increment；空→~5s 超时关，优雅断连，实测） |

## 角色
- **不是产品交付端**（产品端是 native Tauri 壳 + Flutter FFI 壳），是**联调/冒烟宿主**。
- 全 env 配置：`HELIX_WS_URL`(默认 `ws://127.0.0.1:8065/api/v4/websocket`) · `HELIX_API_BASE`(默认 `http://127.0.0.1:8065/api/cses`) · `HELIX_COOKIE_ID`(=userId，鉴权主键) · `HELIX_COMPANY_ID` · `HELIX_DB`(tenant SQLite 路径)。
- 现网 Go 在 WS 升级阶段按 `cookieId=userId` 建 session；session 空则不发 hello、~5s 超时关（实测验证）。

## 关联
- 全景：[`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) M2 D1-2「最小 native 宿主」✅
- engine 泵真源：[`helix-driver-native/docs/design`](../../../helix-driver-native/docs/design/)
