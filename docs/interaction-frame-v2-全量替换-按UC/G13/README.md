# G13 InteractionFrame V2 已切换

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 业务 UC | G13 |
| 机器 Gate | G13 |
| 用户目标 | 更新频道已读边界和未读摘要 |
| 用户角色 | 阅读用户 |
| 观察者角色 | 不适用 |
| Angular 入口 | 频道可见或点击已读 |
| ActionBinding 或观察输入 | `ActionBinding(channel_read)` / `HostObservation` |
| Helix Command | `im_channels_view` |
| 主要 Tick | `Tick::Command`、`Tick::Inbound`、`Tick::PortReply` 或 `Tick::Timer`，按本 UC 输入选择 |
| 主要 Effect | `http_write`、`storage_upsert`、`projection` |
| PersistAtomic 写集 | `channel`、`message_receipt` |
| 当前 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2`，V1 已物理删除 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2` |
| 涉及 Surface | `message.left-dialog-list`、`message.unread-summary`、`message.timeline` |
| Angular 组件 | message-ui-surface-dialog-list、message-read-reporting |
| Go HTTP | `POST /api/cses/post/read` body `{channelId,endTime?,startTime?,posts?}`；会话级已读 `POST /api/cses/channels/view` body `{channels:[{id,isRoot}]}` |
| Go WS | `post_read`, `update_channel`; 离线 sync 可见 `channel_event(type=6)` |
| 远端 PG | `post`、`channel_member`、`channel`、`channel_event` |
| 本地 SQLite | `channel`、`message_receipt` |
| 当前现状 | Helix 仅生产 `InteractionFrame/v2`，cses-client 仅消费 V2；V1 producer/reader/normalize/fixture 已物理删除 |
| 目标状态 | Helix 在匹配持久化屏障后组装完整 V2 VM，Angular 只做协议校验、原子缓存与渲染 |
| 迁移缺口 | 无；删除后回归与文档收口已完成 |
| 验收状态 | V1 已删除；删除后 25-Gate 回归执行中 |

## 完整路径摘要

阅读用户输入或服务端事实
→ Angular/Tauri
→ `ActionBinding(channel_read)` / `HostObservation`
→ `Tick`
→ `Effect`
→ Go authority
→ `PersistAtomic`
→ `PersistOk`
→ `SurfaceComposer`
→ `InteractionFrameV2`
→ Angular 整帧验证与原子应用
→ message-ui-surface-dialog-list、message-read-reporting
→ 未读计数与已读边界同帧变化。

## 四段事实

### 当前现状

Helix formal commit `32e3a0659e4f29add1284be2ba1972eb28f0f938` 已删除 InteractionFrame V1 type/codec/writer 与 V1 fixture；cses-client commit `a4770d6d6` 已重钉四个 Helix crate，并在 Native event sink 与 Angular codec 双层只接受 `schemaVersion=2`。Strict Matrix 与生成 consumer 均为 `InteractionFrame/v2`。

### 目标状态

更新频道已读边界和未读摘要 的业务决策归 Helix domain/application；Go 只提供权威 HTTP/WS/PG 事实；Tauri 只传输；Angular adapter 只消费完整 VM，组件只保留输入焦点、抽屉开关和滚动锚点等本地 UI 状态。

### 迁移缺口

生产协议迁移已完成；剩余工作是删除后同一新 candidate 的 25-Gate 连续回归、静态残留扫描与 Spec/Plan/Tasks 收口。

### 证据

- Helix Gate 合同：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:1478`
- Helix V2 唯一 Frame 合同：`crates/helix-im/src/interaction_frame.rs` 与 `crates/helix-im/src/surface_composer.rs`
- Helix V1 删除提交：`32e3a0659e4f29add1284be2ba1972eb28f0f938`
- Angular 原子 apply：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- Go 权威入口：`/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/channel/handlers_create.go:184`
