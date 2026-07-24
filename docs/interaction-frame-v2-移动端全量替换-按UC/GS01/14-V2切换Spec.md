# GS01 V2 切换 Spec

**目标**：连接建立与 `hello` 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
连接完成后，会话列表与未读摘要一次出现；连接受理不能显示为空列表成功。

**输入**：`HostObservation`；**Command**：`im_query_dialog_list`；**Effect**：`local_scan`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.left-dialog-list`、`message.unread-summary`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
