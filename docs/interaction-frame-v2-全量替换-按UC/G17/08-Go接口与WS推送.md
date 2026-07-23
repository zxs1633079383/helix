# G17 Go 接口与 WS 推送

## 结论

G17 不应新增“返回整个左侧列表”的热路径 HTTP。Go 仍负责权威业务事实，Helix 负责把这些事实投影成 viewer-specific dialog delta。

## 初始与恢复

| 场景 | 当前来源 | G17 要求 |
|---|---|---|
| 冷启动/全量恢复 | channel increment 同步与本地 SQLite snapshot | 可以 bounded 全量，但必须有 cursor/end 边界 |
| 陌生频道 | `load/incrementByChannelId` 类 targeted hydration | 只按 `channelId` 补齐，不扫描全部频道 |
| revision gap | 重新请求 snapshot | 不从丢失 delta 猜测业务状态 |

## 在线权威事件

| WS 事件 | 可能影响的 dialog 字段 | G17 投影 |
|---|---|---|
| `increment_channel` | 完整频道行、未读、成员、preview | 新行 `upsertRow`，必要时 `moveRow` |
| `update_channel` | title、preview、badge、置顶、设置 | 现有行 partial upsert，必要时 move |
| `channel_close` | lifecycle/section | upsert lifecycle + move 到 closed，或按 viewer 规则 remove |
| `channel_member_update` | memberCount、viewer membership | upsert；退出后 remove/move |
| `post` / `posts_update` | preview、lastActivityAt、unread | upsert + move + paired unread |
| 话题权威事实 | `topicCount` | upsert topicCount；禁止 Angular 自增 |

## 权威 payload 要求

Go 事件必须提供或允许 Helix 从本地权威状态得到：

- `channelId`
- viewer identity / audience
- 单调 `eventSeq` 或等价 cursor
- 影响排序的绝对态：置顶、生命周期、last activity
- badge 与 topic count 的绝对值，而不是不可重放的 UI 增量
- 创建、关闭、退出等 causation identity

若 WS 事实不足，Helix 只能发起 targeted hydration；不得为了生成一个 row delta 每次调用全量频道列表接口。

## 多方可见性

频道变化通常为 `multi-party`，但 Go 不应广播同一份 viewer-agnostic UI row。Go 发布业务事实和 audience；Helix 按每个 viewer 的成员关系、未读、通知模式和权限投影独立 user scope。

## 当前源码证据

- Go WS 事件枚举：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/event_type.go:28`
- `update_channel` 发布封装：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_channel_tx.go:81`
- 冷启 increment 进入 bulk lane：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn_priority_lane_test.go:103`
- 话题创建当前只发布根消息 `post_update`，父群 `topicCount` 仍需另行补合同：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_maketopic.go:225`
