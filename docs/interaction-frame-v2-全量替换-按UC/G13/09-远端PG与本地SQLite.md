# G13 远端 PostgreSQL 与本地 SQLite

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## 远端 PostgreSQL

| 项目 | 当前事实 |
|---|---|
| database/schema | 默认配置可见，但运行态可能被 Consul/env 覆盖；实际值已阻塞，需只读查询 `current_database(), current_schema()` |
| table | `post`、`channel_member`、`channel`、`channel_event` |
| 主键/唯一键 | 逐表 schema 为准；`channel_event` 物理主键 `(channel_id,id)`，唯一键 `(channel_id,event_seq)` |
| 查询条件 | 本 UC identity、`channelId`、`postId`、用户身份与 cursor |
| 写入/更新字段 | `post.postRead`, `channel.viewChannels`；`post`, `channel_member`, `channel`, `channel_event` |
| authority 字段 | `event_seq`、服务端 identity、角色/可见性与业务 timestamp |
| temporaryId/serverId | G01/G02/G05 需以 `post.temporary_id` 与 `post.id` 对账；其余 UC 不适用 |
| actor/observer | actor 收到请求结果不代表 observer 可见；observer 必须等待定向 WS 或 sync authority |

证据：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/store/sqlstore/schema/channel_event.go:12`；本 UC Go 路径：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/channel/handlers_create.go:184`。

## 本地 SQLite

| 项目 | 当前/目标合同 |
|---|---|
| table | `channel`、`message_receipt` |
| 主键/唯一键 | 以 Helix storage schema 为准；identity 与 cursor 不得由 Angular 生成 |
| PersistAtomic write-set | `channel`、`message_receipt` |
| 插入/更新字段 | authority fact、identity、cursor、read/reaction/lifecycle 字段 |
| cursor/revision | cursor 属业务同步；Surface revision 属 Frame 应用，两者不可混写 |
| pending/canonical identity | 仅发送类 UC 适用；pending 必须在 canonical echo 后收敛 |
| projector 读取字段 | 只读已提交事实，组装 DialogListVm + UnreadSummaryVm + TimelineWindowVm |
| Persist 失败 | cursor、canonical identity、完整 VM 与可见终态均不得推进 |

## 四阶段判定

1. 远端已接受：仅说明 handler/application 接受请求。
2. 远端已持久化：由 PG 行、事务或 authority event 证明。
3. 本地已 `PersistOk`：由匹配 operation 的本地原子事务证明。
4. Frame 已被 Angular 原子应用：由 matching Surface、revision 和 `FrameApplied` 证明。

四阶段必须分别取证，任何一个阶段都不能替代下一阶段。
