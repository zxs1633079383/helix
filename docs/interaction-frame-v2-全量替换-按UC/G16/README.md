# G16 InteractionFrame V1 到 V2 替换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G16 |
| 机器 Gate | G16、G16b、G16c |
| 用户目标 | 关闭、退出或修改频道治理属性 |
| 用户角色 | 频道管理员或成员 |
| 观察者角色 | 其余频道成员 |
| Angular 入口 | 关闭、退出或保存设置 |
| ActionBinding 或观察输入 | `ActionBinding(close/leave/settings)` |
| Helix Command | `im_channel_close` / `im_channel_leave` / `im_channel_change_info` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `http_write`、`storage_upsert`、`projection`、`transport_broadcast` |
| PersistAtomic 写集 | `channel`、`channel_member` |
| 当前 Frame 版本 | 当前 V1，尚未完成 V2 替换 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.channel-summary`、`message.left-dialog-list`、`message.composer`、`message.member-roster` |
| Angular 组件 | message-ui-group-settings-drawer、message-ui-header、message-surface-runtime-view |
| Go HTTP | `POST /api/cses/channel/close` body `{channelId}` |
| Go WS | `channel_close` |
| 远端 PG | `channel`、`channel_member`、`channel_event` |
| 本地 SQLite | `channel`、`channel_member` |
| 当前现状 | 当前 V1，尚未完成 V2 替换；canonical 投影为 `im:channel:closed` / `channel.row.v1` / `RenderNode/v1`；`im:channel:left` / `channel.row.v1` / `RenderNode/v1`；`im:channel:members` / `member.row.v1` / `RenderNode/v1`；`im:channel:settings-updated` / `channel.row.v1` / `RenderNode/v1` |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 统一 V2 writer、逐 Gate ActionBinding/Observation、`surfaceEpoch` 校验、生产 `FrameApplied` 回传、同一 candidate 证据 |
| 验收状态 | 待验证 |

## 完整路径摘要

频道管理员或成员输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(close/leave/settings)`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-group-settings-drawer、message-ui-header、message-surface-runtime-view
→ 禁用输入、移除频道、成员变化或设置更新。

## 四段事实

### 当前现状

当前 V1，尚未完成 V2 替换。当前机器合同仍记录 `im:channel:closed` / `channel.row.v1` / `RenderNode/v1`；`im:channel:left` / `channel.row.v1` / `RenderNode/v1`；`im:channel:members` / `member.row.v1` / `RenderNode/v1`；`im:channel:settings-updated` / `channel.row.v1` / `RenderNode/v1`。客户端 V1/V2 union 与 legacy normalize 仍可达，证据为 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:486` 和 `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:58`。

### 目标状态

关闭、退出或修改频道治理属性 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

当前尚未证明 G16、G16b、G16c 在同一固定 revision 下只输出 V2，也未证明 `v1Frames=0`、`rawBusinessFallbacks=0`。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1788`
- Helix V1 默认 producer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`
- Helix V2 recovery composer：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_channel.go:164`
