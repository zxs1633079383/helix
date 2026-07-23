# G12 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G12 |
| 机器 Gate | G12 |
| 用户目标 | 打开回复线程并加载回复分支 |
| 用户角色 | 查看或回复用户 |
| 观察者角色 | 线程参与者 |
| Angular 入口 | 打开线程、分支或提交回复 |
| ActionBinding 或观察输入 | `ActionBinding(open_reply_tree/reply)` |
| Helix Command | `im_get_replies` / `im_get_reply_branch` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `http_read`、`projection` |
| PersistAtomic 写集 | `message`、`reply_tree` |
| 当前 Frame 版本 | 当前 V1，尚未完成 V2 替换 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.thread-detail`、`message.timeline` |
| Angular 组件 | message-ui-surface-thread-drawer、message-ui-reply-drawer |
| Go HTTP | `POST /api/cses/posts/getReplies` body `{replyId,pageNumber,pageSize,revoke?}`；`POST /api/cses/posts/getReplyBranch` body `{replyFirstLevelId,pageNumber,pageSize,revoke?}` |
| Go WS | 无直接 WS |
| 远端 PG | `post` |
| 本地 SQLite | `message`、`reply_tree` |
| 当前现状 | 当前 V1，尚未完成 V2 替换；canonical 投影为 `im:channel:replies` / `reply.tree.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

## 完整路径摘要

查看或回复用户输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(open_reply_tree/reply)`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-surface-thread-drawer、message-ui-reply-drawer
→ 完整 root/replies 图与回复终态。

## 四段事实

### 当前现状

当前 V1，尚未完成 V2 替换。当前机器合同仍记录 `im:channel:replies` / `reply.tree.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

打开回复线程并加载回复分支 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 G12 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1380`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_read.go:62`
