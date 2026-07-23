# G15 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G15 |
| 机器 Gate | G15、G15a |
| 用户目标 | 创建话题或新群聊并建立首批 Surface |
| 用户角色 | 创建者 |
| 观察者角色 | 新频道成员 |
| Angular 入口 | 提交话题或群聊创建表单 |
| ActionBinding 或观察输入 | `ActionBinding(make_topic/create_channel)` |
| Helix Command | `im_make_topic` / `im_create_channel` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `http_write`、`storage_upsert`、`projection` |
| PersistAtomic 写集 | `channel`、`message`、`reply_tree` |
| 当前 Frame 版本 | 当前 V1，尚未完成 V2 替换 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` |
| Angular 组件 | message-ui-create-topic、message-ui-create-chat、message-surface-runtime-view |
| Go HTTP | `POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位 |
| Go WS | `channel_created`, `post_update` |
| 远端 PG | `channel`、`channel_member`、`channel_topic`、`post`、`channel_event` |
| 本地 SQLite | `channel`、`channel_member`、`message`、`reply_tree` |
| 当前现状 | 当前 V1，尚未完成 V2 替换；canonical 投影为 `im:channel:created` / `channel.row.v1` / `RenderNode/v1`；`im:channel:replies` / `reply.tree.v1` / `RenderNode/v1`；`im:channel:created` / `channel.row.v1` / `RenderNode/v1`；`im:channel:members` / `member.row.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

## 完整路径摘要

创建者输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(make_topic/create_channel)`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-create-topic、message-ui-create-chat、message-surface-runtime-view
→ 新话题/频道持久化后进入列表。

## 四段事实

### 当前现状

当前 V1，尚未完成 V2 替换。当前机器合同仍记录 `im:channel:created` / `channel.row.v1` / `RenderNode/v1`；`im:channel:replies` / `reply.tree.v1` / `RenderNode/v1`；`im:channel:created` / `channel.row.v1` / `RenderNode/v1`；`im:channel:members` / `member.row.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

创建话题或新群聊并建立首批 Surface 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 G15、G15a 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1720`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_maketopic.go:352`
