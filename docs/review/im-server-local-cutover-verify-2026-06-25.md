# cses-im-server 本机切换验证报告（2026-06-25）

## 结论

新版 `cses-im-server` 在本机接 pre 数据库/Redis/ES，并关闭 Pulsar 走同步 fallback 的范围内，已跑通 Helix IM 接管主链路。

最终 `scripts/host-cli-FULL-uc-e2e.sh` 结果为 `PASS=31 FAIL=0 SKIP=6`。这等价于原基线 `PASS=33 FAIL=0 SKIP=4` 去掉用户明确要求跳过的 UC-8 投票/平均分 2 项；本轮未发现绿转红 UC。

仍不建议直接把“实 Pulsar 模式”一起宣布切换完成：本轮为了本机可控验证使用 `CSES_IM_PULSAR_URL=` 关闭 Pulsar，且发现新版 Go topic 常量与旧版 Mattermost/Go topic suffix 可能不一致，需要单独带真实 Pulsar 再验一次发布/消费链路。

## 环境

| 项 | 实际值 |
|---|---|
| Helix worktree | `/Users/mac17/.codex/worktrees/dd1f/helix` |
| Go server worktree | `/private/tmp/cses-im-server-cutover-fix` |
| 旧 Go 参考 | `/Users/mac17/workspace/go/mattermost` |
| Go listen | `127.0.0.1:8065` |
| HTTP base | `http://127.0.0.1:8065/api/cses` |
| WS url | `ws://127.0.0.1:8065/api/v4/websocket` |
| DB | `postgres://postgres:***@192.168.6.66:32432/mattermost-imv2` |
| Redis | `redis-cses-pre-redis-cluster-4.redis-cses-pre-redis-cluster-headless.redis-cses:6379` |
| ES | `http://192.168.6.66:32092` |
| Pulsar | 本轮显式关闭：`CSES_IM_PULSAR_URL=` |

Go server 启动后确认：

- `pulsar consumer disabled (empty PulsarURL); CreateCsesPost uses sync fallback`
- `starting websocket hubs number_of_hubs=64`
- `cses-im-server listening addr=:8065`

## 基线 -> 切换 -> 重跑 -> 对账

| 阶段 | 验证 | 结果 |
|---|---|---|
| Helix 离线基线 | `cargo test -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` | PASS |
| HX-C001 | `cargo tree -p helix-core` runtime/platform grep | PASS，未发现 tokio/sqlx/reqwest/web-sys/tauri |
| Go 单测 | `GOCACHE=/private/tmp/cses-im-server-cutover-fix/.gocache go test ./...` | PASS |
| WS hello/post 帧 | full UC 脚本双持久连接 | PASS，hello / post / pong 主路径可见 |
| 新版 full UC | `HELIX_WS_URL=ws://127.0.0.1:8065/api/v4/websocket HELIX_API_BASE=http://127.0.0.1:8065/api/cses bash scripts/host-cli-FULL-uc-e2e.sh` | `PASS=31 FAIL=0 SKIP=6` |

最终 UC 对账：

| 类别 | 数量 | 说明 |
|---|---:|---|
| PASS | 31 | 建群、发消息、文档消息、emoji、urgent、已读、话题、定位、回复、撤回、群信息、置顶、成员/角色/昵称、pong、退群等主链路通过 |
| FAIL | 0 | 无绿转红 |
| SKIP | 6 | UC-8 投票/平均分 2 项按用户要求跳过；UC-5.7 status_change 缺状态数据；UC-1.7/UC-4.3 为 harness gap；UC-1.3 上传无 host-cli 管线；UC-1.6 编辑非当前能力 |

## 已定位并修复的新版 Go 问题

| ID | 问题 | 根因分类 | 处理 |
|---|---|---|---|
| R1 | Helix 收到未知 server->client `action:"ping"` 后中断 | server 行为变更 | server ping 改为 opt-in，默认不发，恢复旧 Mattermost WS 合同 |
| R2 | `CSES_IM_PULSAR_URL=` 仍回落默认 `localhost:6650` | server 配置 bug | config 支持显式空值关闭 Pulsar，`main.go` 在 producer nil 时走同步 fallback |
| R3 | sync gap 中 `FilterMemberChannelIds` 偶发 EOF 返回 failed | 数据态/连接瞬断 | SQL store 加 transient EOF retry，避免单次 EOF 直接 fail-close |
| R4 | 新建群后 WS filter 看不到新成员关系 | server 行为 bug | membership cache miss 时回源刷新，StorePresence 正确接入 channel member store |
| R5 | full UC 在 8s 左右双 WS 断开 | wire bug | 诊断日志确认 `websocket: read limit exceeded`；Helix sync-v2 ping 携 150+ root cursors 超过 8KB，新版 read limit 提到 1MiB，并加 300 cursor 回归测试 |
| R6 | 发消息编排在 `IsSameAsLastPost` 崩溃 | server 行为 bug | 对齐旧 Go 逻辑，取 channel last post 而不是走需要 nil 参数保护的 post store 路径 |

## 6 条 curl 契约 diff

curl 使用 `真机curl真源.md` 的 headers/body 形态，对新版本机 server 各打一遍。`posts/create` 因响应不返回 postId，后续 `makeTopic/revoke` 额外使用显式 `id=curlpost000000000000000001` 作为链路锚点；这只用于本轮 chaining，不代表旧真源 body 需要带 id。

| 接口 | HTTP | 新版响应 body | header/结构漂移 |
|---|---:|---|---|
| `POST /channel/create` | 200 | `{"status":"SUCCESS","data":{...,"displayName":"helix-curl-cutover",...}}` | `Cses-Track-Id` 回显；结构为 SUCCESS + channel data |
| `POST /posts/create` | 200 | `{"status":"SUCCESS","message":"post sender success"}` | `Cses-Track-Id` 回显；响应无 postId/data，仍依赖 WS `post` 回声或调用方已知 id |
| `POST /posts/makeTopic` | 200 | `{"status":"SUCCESS"}` | `Cses-Track-Id` 回显；结构与旧真源期望一致 |
| `POST /posts/revoke` | 200 | `{"status":"SUCCESS","message":"revoke post success"}` | 无阻断漂移 |
| `POST /channel/member/change` | 200 | `{"status":"SUCCESS","message":"channelMemberChange success"}` | 无阻断漂移 |
| `POST /channel/close` | 200 | `{"status":"SUCCESS"}` | 无阻断漂移 |

结论：在本机同步 fallback 模式下，6 条写接口未发现 body/header 级阻断漂移。`posts/create` “HTTP 只回 SUCCESS、不回 postId”是已知合同形态，Helix 仍需以 WS `post` 或调用侧 id 完成对账。

## 回归清单

| UC | 基线 | 新版 | 归因 | 建议 |
|---|---:|---:|---|---|
| UC-1.1 普通发消息 | PASS | PASS | 无回归 | 保持 |
| UC-1.2 文档消息 | PASS | PASS | 无回归 | 保持 |
| UC-1.5 撤回 | PASS | PASS | 无回归 | 保持 |
| UC-1.8 emoji | PASS | PASS | 无回归 | 保持 |
| UC-1.9 urgent | PASS | PASS | 无回归 | 保持 |
| UC-2.3 定位 | PASS | PASS | 无回归 | 保持 |
| UC-2.4 回复 | PASS | PASS | 无回归 | 保持 |
| UC-3.2 已读 | PASS | PASS | 无回归 | 保持 |
| UC-4.4 pong | PASS | PASS | 无回归 | 保持 |
| UC-5.1 建群 | PASS | PASS | 无回归 | 保持 |
| UC-5.2 话题 | PASS | PASS | 无回归 | 保持 |
| UC-5.3 退/关群 | PASS | PASS | 无回归 | 保持 |
| UC-5.4 群信息 | PASS | PASS | 无回归 | 保持 |
| UC-5.5 置顶 | PASS | PASS | 无回归 | 保持 |
| UC-6.1 成员 | PASS | PASS | 无回归 | 保持 |
| UC-6.2 角色 | PASS | PASS | 无回归 | 保持 |
| UC-6.3 昵称 | PASS | PASS | 无回归 | 保持 |
| UC-8 投票/平均分 | PASS | SKIP | 用户要求本轮跳过 | 后续如要切投票能力，单独恢复这两项验证 |

本轮没有绿转红 UC，因此没有需要 Helix 侧立即提 PR 的 wire-bug。实际修复点均在新版 Go worktree。

## 剩余风险

1. 实 Pulsar 模式未完成切换验收。本轮关闭 Pulsar 使用同步 fallback；新版 `internal/infra/pulsar/topic.go` 与旧 Go topic suffix/环境前缀可能不一致，切真实 broker 前必须单独验发布和消费。
2. `UC-5.7 status_change` 仍依赖真实状态数据，本轮为数据态 SKIP。
3. `UC-1.7`、`UC-4.3` 是 host-cli harness 能力缺口，不代表新版 server 必然缺能力。
4. Go 侧新增的 WS 诊断日志用于定位 read limit，建议合并前确认日志级别和噪音是否符合生产要求。

