# G01 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-01` | `POST /api/cses/posts/create` body `{viewers,message,mentions,temporaryId,type,simpleMessage,channelId,userId,teamId,userSnapshot,id,props,topicId,revoke,createAt,replyId?,replyRootId?,replyFirstLevelId?}`；`replied={id,reply_id,reply_root_id,reply_first_level_id}` 只在 Helix 组装回复三锚，`req_id` 只透传 `Cses-Track-Id`，两者均不作为 HTTP body 字段 |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-01` | 正常发送主推 `post`；`update_channel` 不是普通发送主链路单独推送，仅在读标记/频道变更/撤回最后一条/加急等副链出现；错误本地 `im:post:send-failed` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
