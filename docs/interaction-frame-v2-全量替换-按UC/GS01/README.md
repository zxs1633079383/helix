# GS01 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | GS01 |
| 机器 Gate | GS01 |
| 用户目标 | 建立连接后完成全量增量恢复 |
| 用户角色 | 重新上线的用户 |
| 观察者角色 | 同账号其他窗口与频道成员 |
| Angular 入口 | 连接建立与 `hello` |
| ActionBinding 或观察输入 | `HostObservation` |
| Helix Command | `im_query_dialog_list` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `local_scan`、`projection` |
| PersistAtomic 写集 | `channel`、`channel_member`、`channel_event_cursor` |
| 当前 Frame 版本 | V1/V2 并存；恢复 composer 已有局部 V2，canonical Matrix 仍为 V1 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.left-dialog-list`、`message.unread-summary` |
| Angular 组件 | message.component、message-surface-runtime-view、message-ui-surface-dialog-list |
| Go HTTP | `POST /api/cses/channels/load/increment` body `{timestamp,cursors:[{channelId,fromSeq}]}`；单频道 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}` |
| Go WS | `hello` 建立连接；零个或多个 `increment_channel.data` 承载频道快照，服务端水位读取成功时包含 `lastEventSeq`，根群 cursor 对账时包含 `needSync`；同一连接最后收到 `increment_channel_end` 只证明该批 WS 入队结束，不证明本地持久化或最终投影完成 |
| 远端 PG | `channel`、`channel_member`、`channel_event`、`post` |
| 本地 SQLite | `channel`、`channel_member`、`channel_event_cursor` |
| 当前现状 | V1/V2 并存；恢复 composer 已有局部 V2，canonical Matrix 仍为 V1；canonical 投影为 `im:channels:loaded` / `dialog.list.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

## 完整路径摘要

重新上线的用户输入或服务端事实
→ Angular/Tauri
→ `HostObservation`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message.component、message-surface-runtime-view、message-ui-surface-dialog-list
→ 会话列表与未读摘要。

## 四段事实

### 当前现状

V1/V2 并存；恢复 composer 已有局部 V2，canonical Matrix 仍为 V1。当前机器合同仍记录 `im:channels:loaded` / `dialog.list.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

建立连接后完成全量增量恢复 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 GS01 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:249`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_channel_increment.go:33`
