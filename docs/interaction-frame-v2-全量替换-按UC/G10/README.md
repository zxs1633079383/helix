# G10：首屏窗口与向前补窗

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 细分文档

1. [当前类图](./01-当前类图.mmd)
2. [当前执行流程](./02-当前执行流程.mmd)
3. [用户交互流程](./03-用户交互流程.mmd)
4. [Angular 组件与交互合同](./04-Angular组件与交互合同.md)
5. [Helix 命令与 Frame 合同](./05-Helix命令与Frame合同.md)
6. [Go 接口与 WS 推送](./06-Go接口与WS推送.md)
7. [远端 PG 与本地 SQLite](./07-远端PG与本地SQLite.md)

## G10 字段级主链路与三端验收

8. [全领域字段级类图](./08-G10全领域字段级类图.mmd)
9. [完整主链路时序图](./09-G10完整主链路时序图.mmd)
10. [Frame 阶段与接受值](./10-G10-Frame阶段与接受值.mmd)
11. [字段级数据合同与测试证据](./11-G10字段级数据合同与测试证据.md)
12. [完整主链路总览](./12-G10-完整主链路.mmd)
13. [Helix、PC、Mobile V2 验收合同](./13-G10-Helix-PC-Mobile-V2验收合同.md)
14. [当前 handoff](./handoff.md)

这些文档固定区分 `CURRENT/HISTORICAL/TARGET/BLOCKED/FORBIDDEN`。历史 25-Gate
Native 产物只作 `HISTORICAL` 输入；没有用当前 pin 重新运行时不得写成当前 Green。

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | `G-10`、`G-10a` |
| 用户/系统入口 | 切换频道；上拉更早历史 |
| 入口模式 | Surface attach/resync + direct query |
| Helix command | `im_query_messages_by_channel`、`im_load_older_context` |
| Effect 类别 | `local_scan`、`http_read`、`storage_upsert`、`projection` |
| canonical Frame Surface | `message.timeline` |
| 合同 persistence | `message` |
| MessageV4 组件 | `MessageSurfaceRuntimeViewComponent`、`MessageComponent` |

## 实际调用链

`activateChannel/loadOlder` -> `im_query_messages_by_channel` / `im_load_older_context`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。首屏与补窗都由完整 `TimelineWindowVm` 驱动；older receipt 只触发 authoritative refresh。

## HTTP / WS

- G-10: 条件回源：`POST /api/cses/posts/getLatestPost` body `{channelId,timestamp:0}`；仅 Complete 分支不出网
- G-10a: `POST /api/cses/posts/postContext` body `{postId,before}`

- G-10: 无
- G-10a: 无直接 WS

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:1173`、`crates/helix-im/contracts/strict-matrix.v1.json:1235`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/g10_timeline_window_interaction_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
