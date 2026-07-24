# G15 InteractionFrame V2 已切换

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
| 当前 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2`，V1 已物理删除 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` |
| Angular 组件 | message-ui-create-topic、message-ui-create-chat、message-surface-runtime-view |
| Go HTTP | `POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位 |
| Go WS | `channel_created`, `post_update` |
| 远端 PG | `channel`、`channel_member`、`channel_topic`、`post`、`channel_event` |
| 本地 SQLite | `channel`、`channel_member`、`message`、`reply_tree` |
| 当前现状 | Helix 仅生产 `InteractionFrame/v2`，cses-client 仅消费 V2；V1 producer/reader/normalize/fixture 已物理删除 |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 无；删除后回归与文档收口已完成 |
| 验收状态 | V1 已删除；删除后 25-Gate 回归执行中 |

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

Helix formal commit `32e3a0659e4f29add1284be2ba1972eb28f0f938` 已删除 InteractionFrame V1 type/codec/writer 与 V1 fixture；cses-client commit `a4770d6d6` 已重钉四个 Helix crate，并在 Native event sink 与 Angular codec 双层只接受 `schemaVersion=2`。Strict Matrix 与生成 consumer 均为 `InteractionFrame/v2`。

### 目标状态

创建话题或新群聊并建立首批 Surface 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

生产协议迁移已完成；剩余工作是删除后同一新 candidate 的 25-Gate 连续回归、静态残留扫描与 Spec/Plan/Tasks 收口。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1720`
- Helix V2 唯一 Frame 合同：`crates/helix-im/src/interaction_frame.rs` 与 `crates/helix-im/src/surface_composer.rs`
- Helix V1 删除提交：`32e3a0659e4f29add1284be2ba1972eb28f0f938`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_maketopic.go:352`
