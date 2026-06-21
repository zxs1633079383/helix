# 真机 parity 验收阻塞 — grpc 根因分析（2026-06-21）

> 背景：sync-v2 全 parity re-port 已代码完成 + 确定性验证全绿（277 测试 / gate / holistic review），
> 唯**真机在线+离线验收**跑不通。本文定位根因到底。
> 结论一句话：**不是 helix re-port 的问题，不是 deviceId，不是 DNS——是后端 `xpa-12-grpc` 这个 grpc 服务在 consul 里 0 实例。**

---

## 1. 失败现象

`scripts/dev-loop.sh` 真连 Go（localhost:8065，full re-ported host-cli）：
- WS 连接 ✅ → hello 握手 ✅ → scan cursor ✅ → increment HTTP 触发 ✅
- 然后 Go 日志：
  ```
  app/grpcx9.go:367      error  "no service available"
  app/cses_channel.go:177 error "parallel load failed"  error="new grpc client fail"
  platform/web_conn.go:557 "ws is close from server"     (1006 abnormal closure)
  ```
- helix 侧：只收到 hello + multiple_channels_viewed；**0 个 increment_channel 帧**（Go load 失败没推）。

## 2. 调用链（逐跳证据）

| 跳 | 位置 | 证据 |
|---|---|---|
| 1 | Go `LoadIncrementChannels` → 并行 load group g2 | `cses_channel.go:165-178`（loadTeamMap / LoadChannelMemberCounts）|
| 2 | `loadTeamMap` → `ReadIdNameWithCompany` | `cses_channel.go:447-471` |
| 3 | → `GrpcInvoke(session, body, "post", "/Company/...", CsesGrpc, ctx)` | `grpc_client.go:29/89/110` |
| 4 | **`CsesGrpc = "xpa-12-grpc"`** | `grpcx9.go:66`（67/68 行 `cses-grpc-qh` / `cses-server-paas2-grpc` 被注释）|
| 5 | `NewGrpcClientWithServiceName("xpa-12-grpc")` → `lb.GetRoundRobinBalancer(consul, name)` | `grpcx9.go:151,365` |
| 6 | consul 解析 `xpa-12-grpc` → **0 实例** → err `"no service available"` | `grpcx9.go:367` |
| 7 | 上抛 `errors.New("new grpc client fail")` → `parallel load failed` | `grpcx9.go:368` + `cses_channel.go:177` |

## 3. 根因（确证）

**`xpa-12-grpc` 在 consul 里 0 实例**（注册 0 条，不是不健康，是根本没注册）：

```bash
# consul 地址 = config.ServiceSettings.ConsulURL = http://consul-pre.jinqidongli.com (→192.168.6.110, 可达 200)
curl "http://consul-pre.jinqidongli.com/v1/health/service/xpa-12-grpc?passing=true"   # → []  健康实例 0
curl "http://consul-pre.jinqidongli.com/v1/catalog/service/xpa-12-grpc"                # → []  catalog 0 条
kubectl exec -n consul-cses consul-server-0 -- consul catalog nodes -service=xpa-12-grpc  # No nodes match
# 对比 xpa-grpc(consul-xpa) 同样 0 健康实例
```

**排除的非根因**（逐个证伪）：
- ❌ deviceId：已用真值 `ED380414932542BAB4993E5AC3E07C64`（host-cli 已写死）；错误从「grpc service fail(IsSuccess=false 会话拒)」前移到「no service available(client 创建阶段)」——证明 deviceId 已过。
- ❌ DNS：`consul-pre.jinqidongli.com` 解析 192.168.6.110、curl 200、leader 10.200.8.80:8300 正常。
- ❌ Consul down：`consul-cses/consul-server-0` Running 35d，catalog 正常返回 16+ 服务。
- ❌ helix re-port：链路在 helix 侧全对（WS/握手/scan/increment 触发都成），Go 自身 load 失败。
- ❌ 其它 grpc 服务：`MattermostIm-Grpc`(5) / `cses-server-paas2-grpc`(2) / `cses-grpc`(1) / `mattermost-grpc-im`(2) / `mattermost-grpc-im-local-xpa-12`(1) 都有健康实例。**唯独 `xpa-12-grpc` 与 `xpa-grpc` 是 0。**

**旁证（config 里的 196/192 IP 混乱，非本次根因但需留意）**：
- 本机 LAN IP = `196.168.1.177`（真实，nc 196/192 都通）。
- config `X9CSESGrpcListenAddress: "196.168.1.99:3388"` → `196.168.1.99:3388` **不通**，但 `192.168.1.99:3388` **通**（196/192 写错）。
- 说明 .99 那台 xpa 主机在 192.168.1.99，config 写成了 196。

## 4. 修复选项（后端拓扑决策 · 需用户拍板）

**选项 A（推荐 · 治本）**：把 `xpa-12-grpc` 服务起起来并注册到 consul-pre。
- 该服务 = xpa/paas 的 company grpc（处理 `/Company/readMember` 等），现网 share-12 实例。
- `192.168.1.99:3388` 可达（疑似它的端点），但 consul 里没 `xpa-12-grpc` 注册 → 需在其宿主把该服务拉起 / 修注册（用 `GetLocalIPAddress` 注册时注意别又落 196.x 不可达）。
- 起好后：`curl ".../v1/health/service/xpa-12-grpc?passing=true"` 应 ≥1。

**选项 B（最快 · 治标 · 有数据源风险）**：把 Go 的 `CsesGrpc` 从 `xpa-12-grpc` 改成已在线的 `cses-server-paas2-grpc`（2 健康实例），重编重启 Go。
- `grpcx9.go:66` 改 const（67/68 行已留注释，本是同一逻辑服务的多环境开关）。
- ⚠️ 风险：`cses-server-paas2-grpc` = paas2 集群，与 xpa-12 可能是**不同租户/环境的数据源**。若测试用户(`5ffd465e…`/company `64118eeb…`)的数据在 xpa-12 后端，切 paas2 会读到空/错数据，increment 仍无意义。**仅当确认测试数据在 paas2 才可用。**

## 5. 验证（修复后）

```bash
cd worktrees/sync-v2-report
FOCUS='increment|channel|sync|post|read' bash scripts/dev-loop.sh 18
# 期望：Go 日志无 "parallel load failed"；helix 收到 increment_channel 帧；
# 落库 Effect + emit 流动；再删库验离线 fromSeq=0 backfill。
```

deviceId 已写死（`ED380414932542BAB4993E5AC3E07C64`，env 可覆盖），无需再传。
