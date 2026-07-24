# G04：在线撤回与离线撤回重放

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-04`、`G-04a` |
| 用户/系统入口 | 消息菜单撤回；observer 收到离线 type-3 事实 |
| 入口模式 | actor 使用 ActionBinding；observer 使用 Inbound/sync |
| Helix command | `im_revoke`、`im_sync_channels` |
| Effect 类别 | `http_write`、`storage_upsert`、`projection`、`sync_request` |
| canonical Frame Surface | `message.timeline` |
| 合同 persistence | `message`、`channel_event_cursor` |
| MessageV4 组件 | `MessageUiActionMenuComponent`、`MessageUiSurfaceThreadDrawerComponent`、`MessageSurfaceRuntimeViewComponent` |

## 实际调用链

`requestSurfaceAction` -> `im_invoke_action`；离线事实不经过 UI command

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。actor/observer 都只按提交后的 timeline Frame 显示撤回；raw WS 不直接改 DOM。

## HTTP / WS

- G-04: `POST /api/cses/posts/revoke` body `{postId}`
- G-04a: 无 UI 主动 HTTP；由 `channels/load/increment` / `sync/notify` 拉到事件

- G-04: `posts_update`, `post_update`, `update_channel`
- G-04a: `increment_channel`, `posts_update`, sync events

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:750`、`crates/helix-im/contracts/strict-matrix.v1.json:811`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/g04_revoke_interaction_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/components/__tests__/message-ui-g04-revoke.component.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
