# G16 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-16` | `POST /api/cses/channel/close` body `{channelId}` |
| `G-16b` | `POST /api/cses/channel/member/leave` body `{channelId,userId?,newCreator?}` |
| `G-16c` | `POST /api/cses/channel/change/info` body `{channelId,name?,displayName?,header?,purpose?,orient?,module?,pictureType?,picture?,source?}`；其他专用命令见 UC-5.5；写入不携带客户端版本条件 |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-16` | `channel_close` |
| `G-16b` | `channel_member_update`, `quit_company`（公司域也可能推） |
| `G-16c` | `update_channel`, `update_channel_notice` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
