# GS01：连接建立后的全量增量恢复

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-S1` |
| 用户/系统入口 | 连接建立、Surface attach 或全量恢复 |
| 入口模式 | HostObservation / Surface resync -> direct command |
| Helix command | `im_query_dialog_list` |
| Effect 类别 | `local_scan`、`projection` |
| canonical Frame Surface | `message.left-dialog-list` |
| 合同 persistence | `channel`、`channel_member`、`channel_event_cursor` |
| MessageV4 组件 | `MessageComponent`、`MessageSurfaceRuntimeViewComponent`、`MessageUiSurfaceDialogListComponent` |

## 实际调用链

`ShellService.start/requestInteractionFrameSnapshot` -> `im_query_dialog_list`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。会话列表只在权威快照可用后出现；HTTP 受理或 `increment_channel_end` 不能单独晋升 ready。

## HTTP / WS

- G-S1: `POST /api/cses/channels/load/increment` body `{timestamp,cursors:[{channelId,fromSeq}]}`；单频道 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}`

- G-S1: `hello` 建立连接；零个或多个 `increment_channel.data` 承载频道快照，服务端水位读取成功时包含 `lastEventSeq`，根群 cursor 对账时包含 `needSync`；同一连接最后收到 `increment_channel_end` 只证明该批 WS 入队结束，不证明本地持久化或最终投影完成

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:234`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
