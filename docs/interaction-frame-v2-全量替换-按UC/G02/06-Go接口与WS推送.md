# G02 Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
| `G-02` | `POST /api/cses/posts/create`，`type=DOCUMENT`；canonical message/simpleMessage=`"【文档】 http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15"`，`props.document={version:1,url:"http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15",title:"打开文档",placeholder:true}`；这是文档消息卡片，不是文件附件或上传任务 |

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
| `G-02` | `post` |

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
