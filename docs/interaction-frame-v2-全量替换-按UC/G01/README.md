# G01：发送文本

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 细分文档

1. [当前类图](./01-当前类图.mmd)
2. [当前执行流程](./02-当前执行流程.mmd)
3. [用户交互流程](./03-用户交互流程.mmd)
4. [Angular 组件与交互合同](./04-Angular组件与交互合同.md)
5. [Helix 命令与 Frame 合同](./05-Helix命令与Frame合同.md)
6. [Go 接口与 WS 推送](./06-Go接口与WS推送.md)
7. [远端 PG 与本地 SQLite](./07-远端PG与本地SQLite.md)

## G01 字段级深挖

8. [全领域字段级类图](./08-G01全领域字段级类图.mmd)
9. [发送全链路时序图](./09-G01发送全链路时序图.mmd)
10. [三个 Frame 与状态纠错](./10-G01三个Frame与状态纠错.mmd)
11. [字段级数据合同与测试证据](./11-G01字段级数据合同与测试证据.md)
12. [创建群聊到消息终态的完整主链路](./12-G01-完整主链路.mmd)
13. [Helix-PC-Mobile V2 验收合同](./13-G01-Helix-PC-Mobile-V2验收合同.md)

最重要的当前事实：普通发送入口是 direct `im_send_message`，不是
`ActionBinding(message.create)`；producer-backed direct 成功路径是 `sending` 与
`sent` 两个 `message.timeline` Frame。若随后收到可 apply 的 WS `post`，
`DialogPatchByPost` 还可能产生含 `message.left-dialog-list`（未读变化时配对
`message.unread-summary`）的独立 Frame；它不是固定第三帧，也不能假定与
`sent` 的先后。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-01` |
| 用户/系统入口 | composer 输入文本并提交 |
| 入口模式 | direct command；当前没有 `message.create` ActionBinding |
| Helix command | `im_send_message` |
| Effect 类别 | `storage_upsert`、`http_write`、`timer`、`projection` |
| canonical Frame Surface | 建群前置：`message.left-dialog-list`；HTTP：`message.timeline`；WS：独立的 `message.timeline` 与条件性 `message.left-dialog-list` |
| 合同 persistence | `message`、`pending_send` |
| MessageV4 组件 | `MessageUiComposerComponent`、`MessageSurfaceRuntimeViewComponent` |

## 实际调用链

`MessageComponent.submitComposer` -> `ShellService.sendText` -> `im_send_message`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。当前 canonical Frame 只刷新 timeline；composer draft 属于 shell state，不能把 Dialog/Unread 影响面写成已提交 mutation。

## HTTP / WS

- G-01: `POST /api/cses/posts/create` body `{viewers,message,mentions,temporaryId,type,simpleMessage,channelId,userId,teamId,userSnapshot,id,props,topicId,revoke,createAt,replyId?,replyRootId?,replyFirstLevelId?}`；`replied={id,reply_id,reply_root_id,reply_first_level_id}` 只在 Helix 组装回复三锚，`req_id` 只透传 `Cses-Track-Id`，两者均不作为 HTTP body 字段

- G-01: 正常发送主推 `post`；`update_channel` 不是普通发送主链路单独推送，仅在读标记/频道变更/撤回最后一条/加急等副链出现；错误本地 `im:post:send-failed`

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:477`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 完整回放：`crates/helix-im/tests/g01_full_flow_test.rs`
- Helix/WDIO 跨仓合同：`crates/helix-im/tests/g01_wdio_fixture_contract_test.rs`、
  `crates/helix-im/tests/fixtures/g01-wdio/`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client Fixture/Mock：`tests/e2e/wdio/fixtures/g01/`、
  `tests/e2e/wdio/specs/g01-mock-suite.e2e.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
