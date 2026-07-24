# G15 HTTP、WS、PG 与 SQLite 合同

共享字段真源：

- [Go HTTP/WS](../../interaction-frame-v2-全量替换-按UC/G15/08-Go接口与WS推送.md)
- [PG/SQLite](../../interaction-frame-v2-全量替换-按UC/G15/09-远端PG与本地SQLite.md)

| 面 | 本 Gate 摘要 | Mobile 可见边界 |
|---|---|---|
| HTTP | `POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位 | C++/JS 不直接请求，只收 correlation 结果 |
| WS | `channel_created`, `post_update` | Rust host 持有连接，Mobile 不解析业务 WS |
| PG | `channel`、`channel_member`、`channel_topic`、`post`、`channel_event` | 只读验收证据，不作为 UI 输入 |
| SQLite | `channel`、`message`、`reply_tree` | 由 Rust host 写入，QuickJS 禁止 row query |
| barrier | `PersistAtomic -> PersistOk` | PersistOk 后才允许 durable Frame |

本文件保持完整证据面，但不复制共享字段定义；共享合同变更时必须先更新 PC/Helix 真源，再复核本 Gate。
