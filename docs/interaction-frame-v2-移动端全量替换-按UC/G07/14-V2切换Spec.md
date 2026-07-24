# G07 V2 切换 Spec

**目标**：发起加急或点击确认 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
加急标识、确认人数、目标列表与按钮权限一致，actor/observer 视图按权限分离。

**输入**：`ActionBinding(urgent/urgent_confirm)`；**Command**：`im_urgent_post` / `im_urgent_confirm`；**Effect**：`http_write`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`、`message.participant-receipts`、`message.unread-summary`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
