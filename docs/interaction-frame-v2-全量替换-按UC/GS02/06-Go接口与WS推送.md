# GS02 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-S2` | `POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}`，单次最多 500 个 cursor |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-S2` | HTTP `/channel/sync/notify` 本身不产生业务 WS；在线并发的 `post` / `posts_update` / `increment_channel_end` 必须作为独立 transport evidence，不能替代 HTTP entries、本地持久化或最终投影 |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
