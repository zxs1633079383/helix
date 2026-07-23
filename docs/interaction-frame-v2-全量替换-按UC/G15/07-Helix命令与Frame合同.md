# G15 Helix 命令与 Frame 合同

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## 客户端产生

- UI semantic intent：提交话题或群聊创建表单。
- `SurfaceActionRequested`：`ActionBinding(make_topic/create_channel)`。
- `ActionInvocationEnvelope`：`actionRef`、`input`、`requestId`、`observedRevision`、`surfaceEpoch`。
- 当前 Tauri/Helix 命令：`im_make_topic` / `im_create_channel`。

## Helix 接收与产生

- 接收：`Command`、`HostObservation`、`Inbound`、`PortReply`、`Timer` 中与本 UC 匹配的输入。
- 产生 Effect：`http_write`、`storage_upsert`、`projection`。
- `PersistAtomic` 写集：`channel`、`message`、`reply_tree`；持久 Gate 只有匹配 `PersistOk` 后才可发 durable Frame。
- 产生：`InteractionActionReceipt`、`InteractionFrameV2`、必要的 `UiDirective`。

## 客户端接收与回传

- 接收完整 `InteractionFrameV2` 和 `ActionReceipt`，不能消费半个 VM。
- revision gap、epoch mismatch 或非法 payload 必须 fail-closed，并请求 resync/snapshot。
- 原子提交后回传 `FrameApplied`；directive ack 只能发生在指令执行后。

## Surface 组装

| 项目 | 合同 |
|---|---|
| SurfaceId | `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` |
| SurfaceScope | user、channel、thread 或 receipt scope，按 Surface 类型确定 |
| 完整 VM | TopicListVm + ChannelSummaryVm + DialogListVm + MemberRosterVm |
| snapshot/patch | 首次 attach/resync 用 snapshot；连续 revision 可用 patch |
| surfaceEpoch 来源 | Helix attach/session generation |
| revision/baseRevision | snapshot 建立 revision；patch 的 baseRevision 必须等于当前 revision |
| ActionBinding 所在 VM | 与 `ActionBinding(make_topic/create_channel)` 对应的完整 VM |
| impact policy | 只组装已 attach 且受当前事实影响的 Surface |
| 同帧原子更新 | `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` 中由同一 authority fact 影响的集合 |
| 未 attach | 不生成该 Surface mutation |
| directives | 仅 新话题/频道持久化后进入列表 所需的 reveal、focus、clear 或 detach 语义 |
| FrameApplied 条件 | 所有 mutation 校验并提交，directive 已按提交后顺序执行 |

当前默认 V1 builder 仍位于 `/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/projector/mod.rs:159`；V2 recovery composer 位于 `/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/surface_composer.rs:32`。因此当前不能把正式类型缩写成未定义的“ActionFrame”。
