# G01 HTTP、WS、PG 与 SQLite 合同

共享字段真源：

- [Go HTTP/WS](../../interaction-frame-v2-全量替换-按UC/G01/08-Go接口与WS推送.md)
- [PG/SQLite](../../interaction-frame-v2-全量替换-按UC/G01/09-远端PG与本地SQLite.md)

| 面 | 本 Gate 摘要 | Mobile 可见边界 |
|---|---|---|
| HTTP | `POST /api/cses/posts/create` body `{viewers,message,mentions,temporaryId,type,simpleMessage,channelId,userId,teamId,userSnapshot,id,props,topicId,revoke,createAt,replyId?,replyRootId?,replyFirstLevelId?}`；`replied={id,reply_id,reply_root_id,reply_first_level_id}` 只在 Helix 组装回复三锚，`req_id` 只透传 `Cses-Track-Id`，两者均不作为 HTTP body 字段 | C++/JS 不直接请求，只收 correlation 结果 |
| WS | 正常发送主推 `post`；`update_channel` 不是普通发送主链路单独推送，仅在读标记/频道变更/撤回最后一条/加急等副链出现；错误本地 `im:post:send-failed` | Rust host 持有连接，Mobile 不解析业务 WS |
| PG | `post`、`channel`、`channel_member`、`channel_event` | 只读验收证据，不作为 UI 输入 |
| SQLite | `message`、`pending_send` | 由 Rust host 写入，QuickJS 禁止 row query |
| barrier | `PersistAtomic -> PersistOk` | PersistOk 后才允许 durable Frame |

本文件保持完整证据面，但不复制共享字段定义；共享合同变更时必须先更新 PC/Helix 真源，再复核本 Gate。
