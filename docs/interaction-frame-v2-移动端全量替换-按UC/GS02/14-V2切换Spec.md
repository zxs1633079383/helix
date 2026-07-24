# GS02 V2 切换 Spec

**目标**：`sync notify` 入站 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
恢复期间保留最后一致视图，提交后会话、未读和受影响时间线同帧更新。

**输入**：`Inbound`；**Command**：`im_sync_channels`；**Effect**：`sync_request`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.left-dialog-list`、`message.unread-summary`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
