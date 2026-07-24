# GS03 V2 切换 Spec

**目标**：`pong.gaps` 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
补偿期间显示同步状态但不清空旧视图；合法补偿提交后只刷新受影响 Surface。

**输入**：`Inbound` + `Timer`；**Command**：`im_sync_channels`；**Effect**：`sync_request`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.left-dialog-list`、`message.unread-summary`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
