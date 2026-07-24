# G05 Helix 命令与 Frame 合同

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

## 客户端产生

- UI semantic intent：选择目标并提交转发。
- `SurfaceActionRequested`：`ActionBinding(forward)`。
- `ActionInvocationEnvelope`：`actionRef`、`input`、`requestId`、`observedRevision`、`surfaceEpoch`。
- 当前 Tauri/Helix 命令：`im_relay_messages`。

## Helix 接收与产生

- 接收：`Command`、`HostObservation`、`Inbound`、`PortReply`、`Timer` 中与本 UC 匹配的输入。
- 产生 Effect：`storage_read`、`http_write`、`projection`。
- `PersistAtomic` 写集：`message`、`channel`；持久 Gate 只有匹配 `PersistOk` 后才可发 durable Frame。
- 产生：`InteractionActionReceipt`、`InteractionFrameV2`、必要的 `UiDirective`。

## 客户端接收与回传

- 接收完整 `InteractionFrameV2` 和 `ActionReceipt`，不能消费半个 VM。
- revision gap、epoch mismatch 或非法 payload 必须 fail-closed，并请求 resync/snapshot。
- 原子提交后回传 `FrameApplied`；directive ack 只能发生在指令执行后。

## Surface 组装

| 项目 | 合同 |
|---|---|
| SurfaceId | `message.timeline`、`message.left-dialog-list`、`message.composer` |
| SurfaceScope | user、channel、thread 或 receipt scope，按 Surface 类型确定 |
| 完整 VM | 源/目标 TimelineWindowVm + DialogListVm |
| snapshot/patch | 首次 attach/resync 用 snapshot；连续 revision 可用 patch |
| surfaceEpoch 来源 | Helix attach/session generation |
| revision/baseRevision | snapshot 建立 revision；patch 的 baseRevision 必须等于当前 revision |
| ActionBinding 所在 VM | 与 `ActionBinding(forward)` 对应的完整 VM |
| impact policy | 只组装已 attach 且受当前事实影响的 Surface |
| 同帧原子更新 | `message.timeline`、`message.left-dialog-list`、`message.composer` 中由同一 authority fact 影响的集合 |
| 未 attach | 不生成该 Surface mutation |
| directives | 仅 各目标独立呈现终态 所需的 reveal、focus、clear 或 detach 语义 |
| FrameApplied 条件 | 所有 mutation 校验并提交，directive 已按提交后顺序执行 |

当前正式合同仅为 `InteractionFrameV2`：Helix commit `32e3a0659e4f29add1284be2ba1972eb28f0f938` 已物理删除 V1 type/codec/writer；客户端 commit `a4770d6d6` 仅消费 V2。`ActionInvocationEnvelope` 的独立 schema 版本不属于 Frame V1。
