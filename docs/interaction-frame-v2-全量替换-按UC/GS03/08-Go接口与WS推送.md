# GS03 Go 接口与 WS 推送

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## GS03

### HTTP

- method/path/router/handler/service：`POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}`
- request DTO/字段：以当前 handler binding 的结构体为准，不从旧截图反推。
- response DTO/字段：HTTP 200 application/json；`CommonRes.status=SUCCESS`，`data={entries:[{channelId,kind,events,nextSeq?,resetTo?,messages?}],serverTime}`；`kind` 当前为 `no_change|events|too_long`（`snapshot` 仅保留未启用），单频道 `events` 最多 500 条、gap 大于 5000 返回 `too_long`；`messages` 仅包含 caller 可见正文。Helix 已实现连续 message/channel_event_cursor 同批相关 Persist，HTTP SUCCESS、未 ack 与 Err 均不发布 `im:sync:state`，仅 PortReply Ok 发布 `state=committed` 与 recovery 水位；pong gap 发起的补偿最终额外输出 `trigger=pong_gap`，普通同步不输出 trigger，保持既有 schema。代码证据 `crates/helix-im/tests/gs_persist_barrier_test.rs::gs2_gs3_sync_transaction_contains_message_then_cursor_and_err_stays_silent` / `gs2_gs3_sync_commit_publishes_mutation_dialog_and_sync_state` / `gs3_pong_gap_trigger_survives_http_and_persist_barrier`
- HTTP 200 的准确含义：只表示 `CommonRes` 经 handler 返回；不证明远端持久化、WS 到达、本地 `PersistOk` 或 Frame 应用。公共证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/entity/dto/common_res.go:12`。

### WebSocket

- 事件与 payload：`pong.data` 固定包含 `text=pong`，对账时可含 `gaps:[{channelId,fromSeq,maxSeq}]` 与 `hashMismatch`；普通 pong 可只有 `text`。随后触发的 HTTP `/channel/sync/notify` 本身不产生业务 WS；并发 `post` / `posts_update` / `increment_channel_end` 不得替代补偿 HTTP、本地持久化或最终投影
- producer 证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/presence/heartbeat.go:31`
- 连接 envelope 的 `seq` 是每连接投递序号，不等于 `channel_event.event_seq`；证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn_pump.go:230`。
- 通用 `requestId` 未在 Go 业务 payload 中定位；当前关联键是 `Cses-Track-Id`，进入 `tracing.csesTrackId`。
- 重复、乱序、迟到：远端以 `(channel_id,event_seq)` 唯一约束和 sync cursor 为权威；Helix 负责本地幂等、身份收敛与 revision。
- 断线恢复：短窗可由 128 帧 deadQueue 续传；超窗换连接并走 HTTP sync。


## 已阻塞




- 远端实际 database/schema 未通过只读连接验证，不能把默认配置写成运行态事实。
