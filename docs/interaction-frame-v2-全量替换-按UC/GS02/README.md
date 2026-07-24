# GS02：sync notify 离线恢复

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-S2` |
| 用户/系统入口 | 离线 cursor 补齐或显式同步 |
| 入口模式 | Inbound / Timer / direct command |
| Helix command | `im_sync_channels` |
| Effect 类别 | `sync_request`、`projection` |
| canonical Frame Surface | `message.left-dialog-list` |
| 合同 persistence | `channel_event_cursor`、`message` |
| MessageV4 组件 | `MessageComponent`、`MessageSurfaceRuntimeViewComponent` |

## 实际调用链

`ShellService.recoverSyncGap` -> `im_sync_channels`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。cursor、消息事实和受影响会话 Surface 以同一提交边界收敛。

## HTTP / WS

- G-S2: `POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}`，单次最多 500 个 cursor

- G-S2: HTTP `/channel/sync/notify` 本身不产生业务 WS；在线并发的 `post` / `posts_update` / `increment_channel_end` 必须作为独立 transport evidence，不能替代 HTTP entries、本地持久化或最终投影

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:294`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/gs_persist_barrier_test.rs`
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
