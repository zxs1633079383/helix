# G04a HTTP、WS、PG 与 SQLite 合同

共享字段真源：

- [Go HTTP/WS](../../interaction-frame-v2-全量替换-按UC/G04/08-Go接口与WS推送.md)
- [PG/SQLite](../../interaction-frame-v2-全量替换-按UC/G04/09-远端PG与本地SQLite.md)

| 面 | 本 Gate 摘要 | Mobile 可见边界 |
|---|---|---|
| HTTP | 无 UI 主动 HTTP；由 `channels/load/increment` / `sync/notify` 拉到事件 | C++/JS 不直接请求，只收 correlation 结果 |
| WS | `increment_channel`, `posts_update`, sync events | Rust host 持有连接，Mobile 不解析业务 WS |
| PG | `channel_event`、`post` | 只读验收证据，不作为 UI 输入 |
| SQLite | `message`、`channel_event_cursor` | 由 Rust host 写入，QuickJS 禁止 row query |
| barrier | `PersistAtomic -> PersistOk` | PersistOk 后才允许 durable Frame |

本文件保持完整证据面，但不复制共享字段定义；共享合同变更时必须先更新 PC/Helix 真源，再复核本 Gate。
