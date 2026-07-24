# GS03 HTTP、WS、PG 与 SQLite 合同

共享字段真源：

- [Go HTTP/WS](../../interaction-frame-v2-全量替换-按UC/GS03/08-Go接口与WS推送.md)
- [PG/SQLite](../../interaction-frame-v2-全量替换-按UC/GS03/09-远端PG与本地SQLite.md)

| 面 | 本 Gate 摘要 | Mobile 可见边界 |
|---|---|---|
| HTTP | `POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}` | C++/JS 不直接请求，只收 correlation 结果 |
| WS | `pong.data` 固定包含 `text=pong`，对账时可含 `gaps:[{channelId,fromSeq,maxSeq}]` 与 `hashMismatch`；普通 pong 可只有 `text`。随后触发的 HTTP `/channel/sync/notify` 本身不产生业务 WS；并发 `post` / `posts_update` / `increment_channel_end` 不得替代补偿 HTTP、本地持久化或最终投影 | Rust host 持有连接，Mobile 不解析业务 WS |
| PG | `channel_event`、`post`、`channel` | 只读验收证据，不作为 UI 输入 |
| SQLite | `channel_event_cursor`、`message` | 由 Rust host 写入，QuickJS 禁止 row query |
| barrier | `PersistAtomic -> PersistOk` | PersistOk 后才允许 durable Frame |

本文件保持完整证据面，但不复制共享字段定义；共享合同变更时必须先更新 PC/Helix 真源，再复核本 Gate。
