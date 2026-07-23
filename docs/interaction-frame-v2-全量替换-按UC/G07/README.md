# G07 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G07 |
| 机器 Gate | G07 |
| 用户目标 | 加急消息并收集确认事实 |
| 用户角色 | 加急发起者与确认者 |
| 观察者角色 | 被加急用户 |
| Angular 入口 | 发起加急或点击确认 |
| ActionBinding 或观察输入 | `ActionBinding(urgent/urgent_confirm)` |
| Helix Command | `im_urgent_post` / `im_urgent_confirm` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `http_write`、`projection` |
| PersistAtomic 写集 | `message`、`message_receipt` |
| 当前 Frame 版本 | 当前 V1，尚未完成 V2 替换 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.timeline`、`message.participant-receipts`、`message.unread-summary` |
| Angular 组件 | message-ui-urgent、message-ui-read-receipt、message-surface-runtime-view |
| Go HTTP | `POST /api/cses/posts/urgentPost` body `{channelId,postId,targetIds,message?}`；`POST /api/cses/posts/urgentConfirm` body `{postId,channelId}` |
| Go WS | `post_update`, `update_channel` |
| 远端 PG | `post`、`channel`、`channel_member`、`channel_event` |
| 本地 SQLite | `message`、`message_receipt` |
| 当前现状 | 当前 V1，尚未完成 V2 替换；canonical 投影为 `im:post:updated` / `message.row.v1` / `RenderNode/v1`；`im:post:readers` / `read.receipt.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

## 完整路径摘要

加急发起者与确认者输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(urgent/urgent_confirm)`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-urgent、message-ui-read-receipt、message-surface-runtime-view
→ 加急标识、确认人数与按钮权限。

## 四段事实

### 当前现状

当前 V1，尚未完成 V2 替换。当前机器合同仍记录 `im:post:updated` / `message.row.v1` / `RenderNode/v1`；`im:post:readers` / `read.receipt.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

加急消息并收集确认事实 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 G07 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1005`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_write.go:260`
