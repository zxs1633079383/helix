# G15 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-15` | `POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位 |
| `G-15a` | `POST /api/cses/channel/create` body `{type,userIds,displayName,forceCreate}`；session 身份、team、成员角色与默认 `pictureType=USER/picture.userIds` 由服务端派生，客户端不得提交 team/role/owner/picture |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-15` | `channel_created`, `post_update` |
| `G-15a` | `channel_created` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
