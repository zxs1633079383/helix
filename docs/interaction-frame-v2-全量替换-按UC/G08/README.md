# G08：创建定时消息

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-08` |
| 用户/系统入口 | composer 打开定时对话框并提交 |
| 入口模式 | 当前主要 direct command；代码保留“有 enabled binding 则用”的过渡分支 |
| Helix command | `im_create_schedule` |
| Effect 类别 | `http_write`、`storage_upsert`、`projection` |
| canonical Frame Surface | `message.channel-summary` |
| 合同 persistence | `channel_schedule` |
| MessageV4 组件 | `MessageUiScheduleDialogComponent`、`MessageUiOverlayHostComponent`、`MessageComponent` |

## 实际调用链

`submitSchedule` -> binding（若存在）或 `ShellService.createSchedule` -> `im_create_schedule`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。当前 ActionDispatch 未审计 `createSchedule`；direct 分支通过 bounded dialog resync 等待 Frame。

## HTTP / WS

- G-08: `POST /api/cses/posts/createSchedule` body `{post:{channelId,message,temporaryId?},schedulePostAt}`

- G-08: `post_schedule_created`

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:1068`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/g08_schedule_create_interaction_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/components/__tests__/message-ui-schedule-dialog.component.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
