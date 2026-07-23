# G02 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G02 |
| 机器 Gate | G02 |
| 用户目标 | 发送文档类型消息并按 typed VM 渲染 |
| 用户角色 | 发送者 |
| 观察者角色 | 频道其他成员 |
| Angular 入口 | 选择文档内容并提交 |
| ActionBinding 或观察输入 | `ActionBinding(message.create)` |
| Helix Command | `im_send_message` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `upload`、`storage_upsert`、`http_write`、`projection` |
| PersistAtomic 写集 | `message`、`pending_send` |
| 当前 Frame 版本 | 当前 V1，尚未完成 V2 替换 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.composer`、`message.timeline` |
| Angular 组件 | message-ui-composer、message-document-content |
| Go HTTP | `POST /api/cses/posts/create`，同上，`type=DOCUMENT`；占位字段建议 `message/simpleMessage="【文档】 http://doc.cses7.com/doc/abc123"`，`props.document={version:1,url:"http://doc.cses7.com/doc/abc123",title:"打开文档",placeholder:true}`；后续替换为 message-v3 的 `props.document={snapshot,version:1}` |
| Go WS | `post` |
| 远端 PG | `post`、`channel`、`channel_member`、`channel_event` |
| 本地 SQLite | `message`、`pending_send` |
| 当前现状 | 当前 V1，尚未完成 V2 替换；canonical 投影为 `im:post:received` / `message.row.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

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
→ message-ui-composer、message-document-content
→ 文档消息与失败后的输入恢复。

## 四段事实

### 当前现状

当前 V1，尚未完成 V2 替换。当前机器合同仍记录 `im:post:received` / `message.row.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

发送文档类型消息并按 typed VM 渲染 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 G02 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:567`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_create.go:14`
