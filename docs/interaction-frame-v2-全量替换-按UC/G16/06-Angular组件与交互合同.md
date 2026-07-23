# G16 Angular 组件与交互合同

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

| 字段 | 内容 |
|---|---|
| 组件文件 | `src/pages/message-v4/components/` 下 message-ui-group-settings-drawer、message-ui-header、message-surface-runtime-view |
| 组件 selector | 由上述组件定义；运行时容器为 `message-surface-runtime-view` |
| 输入 input | ChannelSummaryVm + DialogListVm + ComposerVm + MemberRosterVm，以及 Angular 本地焦点/抽屉/滚动状态 |
| 输出 output | 关闭、退出或保存设置 对应的语义事件 |
| 用户事件 handler | 关闭、退出或保存设置 |
| Surface adapter | `message.channel-summary` adapter、`message.left-dialog-list` adapter、`message.composer` adapter、`message.member-roster` adapter |
| 消费 VM | ChannelSummaryVm + DialogListVm + ComposerVm + MemberRosterVm |
| ActionBinding | `ActionBinding(close/leave/settings)` |
| 发送 Command | 当前为 `im_channel_close` / `im_channel_leave` / `im_channel_change_info`；目标由 binding 解析后进入 `im_invoke_action` |
| 接收 Frame | `ProjectionBusService -> ProjectionStore.applyEnvelope -> SurfaceRuntime` |
| 接收 ActionReceipt | 只更新 pending/awaiting-frame/rejected，不把 accepted 当完成 |
| 发送 FrameApplied | 目标必须回传；当前生产注册点未定位 |
| DOM 验收点 | `data-surface`、`data-message-id`、`data-delivery-kind` 及本 UC 控件状态 |
| ARIA 验收点 | 控件名称、disabled、status/alert 与 tab 语义 |
| data-* 验收点 | `data-surface`、`data-revision`、本 UC identity 与状态字段 |
| 当前 V1/V2 状态 | 当前 V1，尚未完成 V2 替换 |
| 迁移缺口 | 逐 Gate 证明 projected binding 或明确 Observation，并接通生产 FrameApplied。 |

## 当前调用链

组件 → output → `MessageComponent` handler → `ShellService` → `im_channel_close` / `im_channel_leave` / `im_channel_change_info`；当前缺口：该路径尚未证明全部使用 projected ActionBinding。

当前页面从 facade 读取九个 VM，证据：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.ts:110`。九 Surface attach 由 `MessageSurfaceFacade` 统一管理，证据：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts:35`。

## 所有权

- Helix 拥有消息排序、权限、未读、回执、身份收敛和 action enablement。
- adapter 只把完整 VM 映射为组件输入，复杂度为每 mutation 一次 `Map` 查找，平均 `O(m)` 时间、`O(m)` 暂存空间。
- 组件只拥有输入焦点、抽屉开关、滚动锚点等短生命周期 UI 状态。
