# G13 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-13` | `POST /api/cses/channels/view` body `{channels:[{id,isRoot}]}` |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-13` | `update_channel` 携带当前 viewer 的 `unreadCount`, `mentionCount`, `urgentCount`, `lastReadSeq` 等 `channel_member` 绝对态；它与 HTTP reply 独立竞态 |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
