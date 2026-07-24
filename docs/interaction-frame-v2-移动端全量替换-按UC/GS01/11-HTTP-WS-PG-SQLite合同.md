# GS01 HTTP、WS、PG 与 SQLite 合同

共享字段真源：

- [Go HTTP/WS](../../interaction-frame-v2-全量替换-按UC/GS01/08-Go接口与WS推送.md)
- [PG/SQLite](../../interaction-frame-v2-全量替换-按UC/GS01/09-远端PG与本地SQLite.md)

| 面 | 本 Gate 摘要 | Mobile 可见边界 |
|---|---|---|
| HTTP | `POST /api/cses/channels/load/increment` body `{timestamp,cursors:[{channelId,fromSeq}]}`；单频道 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}` | C++/JS 不直接请求，只收 correlation 结果 |
| WS | `hello` 建立连接；零个或多个 `increment_channel.data` 承载频道快照，服务端水位读取成功时包含 `lastEventSeq`，根群 cursor 对账时包含 `needSync`；同一连接最后收到 `increment_channel_end` 只证明该批 WS 入队结束，不证明本地持久化或最终投影完成 | Rust host 持有连接，Mobile 不解析业务 WS |
| PG | `channel`、`channel_member`、`channel_event`、`post` | 只读验收证据，不作为 UI 输入 |
| SQLite | `channel`、`channel_member`、`channel_event_cursor` | 由 Rust host 写入，QuickJS 禁止 row query |
| barrier | `PersistAtomic -> PersistOk` | PersistOk 后才允许 durable Frame |

本文件保持完整证据面，但不复制共享字段定义；共享合同变更时必须先更新 PC/Helix 真源，再复核本 Gate。
