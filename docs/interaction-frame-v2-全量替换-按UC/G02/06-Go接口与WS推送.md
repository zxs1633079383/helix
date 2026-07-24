# G02 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-02` | `POST /api/cses/posts/create`，同上，`type=DOCUMENT`；占位字段建议 `message/simpleMessage="【文档】 http://doc.cses7.com/doc/abc123"`，`props.document={version:1,url:"http://doc.cses7.com/doc/abc123",title:"打开文档",placeholder:true}`；后续替换为 message-v3 的 `props.document={snapshot,version:1}` |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-02` | `post` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
