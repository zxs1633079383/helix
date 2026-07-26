# G16：频道关闭、退出与设置

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
| Machine Gate | `G-16`、`G-16b`、`G-16c` |
| 用户/系统入口 | 群设置、关闭/退出确认、属性修改 |
| 入口模式 | direct command；治理 ActionBinding 当前保持 disabled |
| Helix command | `im_channel_close`、`im_channel_leave`、`im_channel_change_display_name`、`im_channel_change_notice`、`im_channel_change_info` |
| Effect 类别 | `http_write`、`storage_upsert`、`projection`、`transport_broadcast` |
| canonical Frame Surface | `message.channel-summary`、`message.member-roster` |
| 合同 persistence | `channel`、`channel_member` |
| MessageV4 组件 | `MessageUiGroupSettingsDrawerComponent`、`MessageUiChannelManagementComponent`、`MessageSurfaceRuntimeViewComponent` |

## 实际调用链

`confirmGovernanceAction/updateGroupSettings` -> `im_channel_close` / `im_channel_leave` / `im_channel_change_*`

`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。频道生命周期、成员 roster 与 settingsVersion 均由 Frame 权威返回；版本冲突不由 UI 猜测覆盖。

## HTTP / WS

- G-16: `POST /api/cses/channel/close` body `{channelId}`
- G-16b: `POST /api/cses/channel/member/leave` body `{channelId,userId?,newCreator?}`
- G-16c: `POST /api/cses/channel/change/info` body `{channelId,name?,displayName?,header?,purpose?,orient?,module?,pictureType?,picture?,source?,expectedVersion?}`；其他专用命令见 UC-5.5

- G-16: `channel_close`
- G-16b: `channel_member_update`, `quit_company`（公司域也可能推）
- G-16c: `update_channel`, `update_channel_notice`

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：`crates/helix-im/contracts/strict-matrix.v1.json:1759`、`crates/helix-im/contracts/strict-matrix.v1.json:1830`、`crates/helix-im/contracts/strict-matrix.v1.json:1886`
- Helix Frame schema：`crates/helix-im/src/interaction_frame.rs`
- Helix 专属测试：`crates/helix-im/tests/g16_governance_interaction_test.rs`
- cses-client command/action：`src/pages/message-v4/shell.service.ts`、
  `src/pages/message-v4/actions/surface-action-invocation.ts`
- cses-client 组件：`src/pages/message-v4/message.component.ts`、
  `src/pages/message-v4/components/message-surface-runtime-view.component.ts`
- cses-client 专属测试：`src/pages/message-v4/components/__tests__/g16-governance-interaction.test.ts`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- `pending_send` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite `message`
  行，不存在 `pending_send` 表。
