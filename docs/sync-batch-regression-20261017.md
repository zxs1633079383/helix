# 有界批量同步回归（2026-10-17）

基线：`reactor/MessageV3-helix`，`b1820ef5`，0.1.71。复用现网
`/api/cses/channel/sync/notify` 多 cursor 协议，不改服务端、存储 schema 或版本。

每轮 drain 将可用窗口内的群合并成一个请求，全局在途群仍最多 8 个。
每个群保留自己的 correlation、cursor、分页、持久化屏障与恢复触发来源。
整批身份与 payload 先校验，缺项、重复项、陌生群、传输失败均不得推进 cursor。
整批子回报处理完才补充窗口；断连清除父子 correlation，旧回报无效。

## 自动回归

`p4_sync_scheduler_test` 10 项通过：20 群分为 3 请求、在途上限、
500 事件分页、pong 优先级、异常批次释放窗口、旧连接回包忽略、
events 与 no_change 混合且只在 PersistOk 后推进对应 cursor、重复回包不重写。
另有 e4_scan_corr_test、increment_flow_test、lifecycle_test、s6_sync_internal_test、
uc44_pong_dispatch_test 及 send/pong_compensate_tests 的请求体与群数断言。

共享内核测试 29 项通过；native/FFI/web/core/im cargo check 和 wasm32 Web 构建通过。
完整 IM 套件发现 7 项无关查询失败，已用 registry 原版 0.1.71 独立重现：
G13A 3 项、G13D 3 项、p6_query_test 1 项。另一个旧 e4 单请求数断言已更新并定向通过。
不得把本次结果标为整仓全绿。

## 真实 Tauri 回归

本地源码 patch 构建候选 SHA256：
`b83cb62ea3743fc2fb7eb54be4961f20251cc8bcd84fec8b12fbd354d9d547c4`。
构建后源码仅格式化与注释调整，无行为变更。客户端原 binary、依赖声明和 Cargo.lock 保持原样。

用独立数据目录运行接收端和发送端，均连接真实 pre 服务。
原专用验收群 `7t5k7rioofbu9da5nt9zn4tbyy`；仅发送用户授权的两条带“批量同步验收”标记的消息。

1. 接收端退出并确认进程及 WebDriver 端口关闭，发送 offline 消息，然后用同一 profile/DB 重启。
   实际聊天区、会话摘要和持久化查询均命中新消息；独立 SQLite 查验 id 只有一条，event_seq=2253。
2. 接收端保持运行，通过专属 HTTP/WS 代理切断其连接，发送 reconnect 消息。
   恢复前独立 SQLite 确认新 id 数量为 0、cursor=2253；恢复代理后由原生连接自动恢复。
   聊天区、摘要与持久化均补到同一 id，仅一条，event_seq=2254。
   从恢复后开始等待至画面确认约 9.2 秒，含既有连接恢复调度；不声称重连一秒内完成。
3. 无新消息，再以同一 profile/DB 重启：223 群检查共 28 HTTP（27×8 + 1×7），
   相比旧逐群 223 请求减少约 87.4%。重启前后 4552 条消息的 id/event_seq 与
   1499 条群 cursor/terminal cursor 完全一致。没有以省略检查换取请求数下降。

证据在客户端 `artifacts/summary-batch-sync-20261017/`：
`receiver/{offline,reconnect}-receive.json`、`reconnect-offline-proof.json`、
`warm-result.json`、截图、`proxy-requests.jsonl`、`test-summary.json`、`baseline-query.log`。
自动化脚本同目录；复跑发送必须重新授权，已有 send-attempt 文件防止重复发送。
首次代理 WS 上游配置错误及重启脚本生命周期问题在发送前/重启时修正；同 profile 数据未重建。

范围：真实 pre 桌面验证，不代表 5000 人大群并行压力验收，也不代表 Flutter/Web 实机验证。
未发布新 Helix 制品；正式客户端仍引用 registry 0.1.71，升级版本和交付另行确认。
