# GS01 Go 接口与 WS 推送

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## GS01

### HTTP

- method/path/router/handler/service：`POST /api/cses/channels/load/increment` body `{timestamp,cursors:[{channelId,fromSeq}]}`；单频道 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}`
- request DTO/字段：以当前 handler binding 的结构体为准，不从旧截图反推。
- response DTO/字段：HTTP 200 application/json；`CommonRes.status=SUCCESS` 且 `data=null` 仅表示异步 worker 已受理，不表示同步完成；Helix 已实现等待同一 WS 连接的 `increment_channel_end`，并将本地 `channel` → `channel_member` → `channel_event_cursor` 合入同一相关 Persist；仅 PortReply Ok 发布最终 `im:channels:loaded` 投影。代码证据 `crates/helix-im/tests/gs_persist_barrier_test.rs` 的 `gs1_http_and_increment_end_do_not_publish_before_same_batch_commit` / `gs1_persist_error_never_publishes_final_projection` / `gs1_persist_ok_publishes_increment_then_loaded`
- HTTP 200 的准确含义：只表示 `CommonRes` 经 handler 返回；不证明远端持久化、WS 到达、本地 `PersistOk` 或 Frame 应用。公共证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/entity/dto/common_res.go:12`。

### WebSocket

- 事件与 payload：`hello` 建立连接；零个或多个 `increment_channel.data` 承载频道快照，服务端水位读取成功时包含 `lastEventSeq`，根群 cursor 对账时包含 `needSync`；同一连接最后收到 `increment_channel_end` 只证明该批 WS 入队结束，不证明本地持久化或最终投影完成
- producer 证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_channel_increment.go:33`
- 连接 envelope 的 `seq` 是每连接投递序号，不等于 `channel_event.event_seq`；证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn_pump.go:230`。
- 通用 `requestId` 未在 Go 业务 payload 中定位；当前关联键是 `Cses-Track-Id`，进入 `tracing.csesTrackId`。
- 重复、乱序、迟到：远端以 `(channel_id,event_seq)` 唯一约束和 sync cursor 为权威；Helix 负责本地幂等、身份收敛与 revision。
- 断线恢复：短窗可由 128 帧 deadQueue 续传；超窗换连接并走 HTTP sync。


## 已阻塞




- 远端实际 database/schema 未通过只读连接验证，不能把默认配置写成运行态事实。
