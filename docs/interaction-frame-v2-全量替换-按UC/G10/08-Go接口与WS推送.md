# G10 Go 接口与 WS 推送

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## G10

### HTTP

- method/path/router/handler/service：条件回源：`POST /api/cses/posts/getLatestPost` body `{channelId,timestamp:0}`；仅 Complete 分支不出网
- request DTO/字段：以当前 handler binding 的结构体为准，不从旧截图反推。
- response DTO/字段：回源分支要求 HTTP 200 application/json 且 CommonRes.status=SUCCESS；完整 message/data 必须入证据包，并与 Helix 投影及远端事实对账；Complete 分支必须以 trace 与 HTTP 计数反证零出网
- HTTP 200 的准确含义：只表示 `CommonRes` 经 handler 返回；不证明远端持久化、WS 到达、本地 `PersistOk` 或 Frame 应用。公共证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/entity/dto/common_res.go:12`。

### WebSocket

- 事件与 payload：无
- producer 证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_read.go:104`
- 连接 envelope 的 `seq` 是每连接投递序号，不等于 `channel_event.event_seq`；证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn_pump.go:230`。
- 通用 `requestId` 未在 Go 业务 payload 中定位；当前关联键是 `Cses-Track-Id`，进入 `tracing.csesTrackId`。
- 重复、乱序、迟到：远端以 `(channel_id,event_seq)` 唯一约束和 sync cursor 为权威；Helix 负责本地幂等、身份收敛与 revision。
- 断线恢复：短窗可由 128 帧 deadQueue 续传；超窗换连接并走 HTTP sync。

## G10a

### HTTP

- method/path/router/handler/service：`POST /api/cses/posts/postContext` body `{postId,before}`
- request DTO/字段：以当前 handler binding 的结构体为准，不从旧截图反推。
- response DTO/字段：HTTP 200 application/json；CommonRes.status=SUCCESS；完整 message/data 必须入证据包，并与 Helix 投影及远端事实对账
- HTTP 200 的准确含义：只表示 `CommonRes` 经 handler 返回；不证明远端持久化、WS 到达、本地 `PersistOk` 或 Frame 应用。公共证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/entity/dto/common_res.go:12`。

### WebSocket

- 事件与 payload：无直接 WS
- producer 证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_read.go:38`
- 连接 envelope 的 `seq` 是每连接投递序号，不等于 `channel_event.event_seq`；证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn_pump.go:230`。
- 通用 `requestId` 未在 Go 业务 payload 中定位；当前关联键是 `Cses-Track-Id`，进入 `tracing.csesTrackId`。
- 重复、乱序、迟到：远端以 `(channel_id,event_seq)` 唯一约束和 sync cursor 为权威；Helix 负责本地幂等、身份收敛与 revision。
- 断线恢复：短窗可由 128 帧 deadQueue 续传；超窗换连接并走 HTTP sync。


## 已阻塞




- 远端实际 database/schema 未通过只读连接验证，不能把默认配置写成运行态事实。
