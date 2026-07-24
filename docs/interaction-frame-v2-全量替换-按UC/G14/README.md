# G14：单条已读与模板已收到

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 细分文档

1. [当前类图](./01-当前类图.mmd)
2. [当前执行流程](./02-当前执行流程.mmd)
3. [用户交互流程](./03-用户交互流程.mmd)
4. [Angular 组件与交互合同](./04-Angular组件与交互合同.md)
5. [Helix 命令与 Frame 合同](./05-Helix命令与Frame合同.md)
6. [Go 接口与 WS 推送](./06-Go接口与WS推送.md)
7. [远端 PG 与本地 SQLite](./07-远端PG与本地SQLite.md)

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-14`、`G-14a` |
| 用户/系统入口 | 单条可见上报；模板确认已收到 |
| 入口模式 | direct command；receipt popover 查询可使用独立 binding |
| Helix command | `im_post_read`、`im_template_received` |
| Effect 类别 | `http_write`、`storage_upsert`、`projection` |
| canonical Frame Surface | `message.participant-receipts`、`message.timeline` |
| 合同 persistence | `message_receipt` |
| MessageV4 组件 | `MessageUiReadReceiptComponent`、`MessageSurfaceRuntimeViewComponent`、`MessageComponent` |

## 实际调用链

`markMessageRead/markTemplateReceived` -> `im_post_read` / `im_template_received`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。已读参与者与模板状态分别进入 participant-receipts/timeline Frame，不能混成一个本地 bool。

## HTTP / WS

- G-14: `POST /api/cses/post/read` body `{channelId,posts:[postId]}`
- G-14a: `POST /api/cses/post/templateReceived` body `{postId}`；注意 Helix 入参读 camel `postId`

- G-14: `post_read`
- G-14a: `post_update`

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:1525`、`crates/helix-im/contracts/strict-matrix.v1.json:1581`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/g14_receipt_template_interaction_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/components/__tests__/g14-receipt-template-interaction.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
