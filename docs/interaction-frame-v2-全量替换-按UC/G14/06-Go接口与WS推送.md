# G14 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-14` | `POST /api/cses/post/read` body `{channelId,posts:[postId]}` |
| `G-14a` | `POST /api/cses/post/templateReceived` body `{postId}`；注意 Helix 入参读 camel `postId` |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-14` | `post_read` |
| `G-14a` | `post_update` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
