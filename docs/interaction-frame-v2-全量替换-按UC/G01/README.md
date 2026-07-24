# G01 InteractionFrame V2 已切换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G01 |
| 机器 Gate | G01 |
| 用户目标 | 发送文本并收敛临时身份与服务端身份 |
| 用户角色 | 发送者 |
| 观察者角色 | 频道其他成员 |
| Angular 入口 | 输入文本并提交 |
| ActionBinding 或观察输入 | `ActionBinding(message.create)` |
| Helix Command | `im_send_message` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `storage_upsert`、`http_write`、`timer`、`projection` |
| PersistAtomic 写集 | `message`、`pending_send` |
| 当前 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2`，V1 已物理删除 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.composer`、`message.timeline`、`message.left-dialog-list`、`message.unread-summary` |
| Angular 组件 | message-ui-composer、message-surface-runtime-view、message-ui-surface-dialog-list |
| Go HTTP | `POST /api/cses/posts/create` body `{viewers,message,mentions,temporaryId,type,simpleMessage,channelId,userId,teamId,userSnapshot,id,props,topicId,revoke,createAt,replyId?,replyRootId?,replyFirstLevelId?}`；`replied={id,reply_id,reply_root_id,reply_first_level_id}` 只在 Helix 组装回复三锚，`req_id` 只透传 `Cses-Track-Id`，两者均不作为 HTTP body 字段 |
| Go WS | 正常发送主推 `post`；`update_channel` 不是普通发送主链路单独推送，仅在读标记/频道变更/撤回最后一条/加急等副链出现；错误本地 `im:post:send-failed` |
| 远端 PG | `post`、`channel`、`channel_member`、`channel_event` |
| 本地 SQLite | `message`、`pending_send` |
| 当前现状 | Helix 仅生产 `InteractionFrame/v2`，cses-client 仅消费 V2；V1 producer/reader/normalize/fixture 已物理删除 |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 无；删除后回归与文档收口已完成 |
| 验收状态 | V1 已删除；删除后 25-Gate 回归执行中 |

## 完整路径摘要

发送者输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(message.create)`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-composer、message-surface-runtime-view、message-ui-surface-dialog-list
→ 同一行从发送中收敛到已发送或失败。

## 四段事实

### 当前现状

Helix formal commit `32e3a0659e4f29add1284be2ba1972eb28f0f938` 已删除 InteractionFrame V1 type/codec/writer 与 V1 fixture；cses-client commit `a4770d6d6` 已重钉四个 Helix crate，并在 Native event sink 与 Angular codec 双层只接受 `schemaVersion=2`。Strict Matrix 与生成 consumer 均为 `InteractionFrame/v2`。

### 目标状态

发送文本并收敛临时身份与服务端身份 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

生产协议迁移已完成；剩余工作是删除后同一新 candidate 的 25-Gate 连续回归、静态残留扫描与 Spec/Plan/Tasks 收口。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:492`
- Helix V2 唯一 Frame 合同：`crates/helix-im/src/interaction_frame.rs` 与 `crates/helix-im/src/surface_composer.rs`
- Helix V1 删除提交：`32e3a0659e4f29add1284be2ba1972eb28f0f938`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_create.go:14`
