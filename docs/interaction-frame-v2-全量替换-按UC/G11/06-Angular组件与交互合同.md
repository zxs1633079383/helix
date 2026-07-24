# G11 Angular 组件与交互合同

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

| 字段 | 内容 |
|---|---|
| 组件文件 | `src/pages/message-v4/components/` 下 message-surface-runtime-view、scroll-anchor-store |
| 组件 selector | 由上述组件定义；运行时容器为 `message-surface-runtime-view` |
| 输入 input | TimelineWindowVm + RevealMessage directive，以及 Angular 本地焦点/抽屉/滚动状态 |
| 输出 output | 请求定位 `postId` 对应的语义事件 |
| 用户事件 handler | 请求定位 `postId` |
| Surface adapter | `message.timeline` adapter |
| 消费 VM | TimelineWindowVm + RevealMessage directive |
| ActionBinding | `ActionBinding(locate)` |
| 发送 Command | 当前为 `im_get_posts`；目标由 binding 解析后进入 `im_invoke_action` |
| 接收 Frame | `ProjectionBusService -> ProjectionStore.applyEnvelope -> SurfaceRuntime` |
| 接收 ActionReceipt | 只更新 pending/awaiting-frame/rejected，不把 accepted 当完成 |
| 发送 FrameApplied | 生产 bridge 已回传 `FrameApplied(transactionId, slots, result)` |
| DOM 验收点 | `data-surface`、`data-message-id`、`data-delivery-kind` 及本 UC 控件状态 |
| ARIA 验收点 | 控件名称、disabled、status/alert 与 tab 语义 |
| data-* 验收点 | `data-surface`、`data-revision`、本 UC identity 与状态字段 |
| 当前 Frame 版本状态 | 仅 V2；V1 consumer/normalize 已物理删除 |
| 迁移缺口 | 无生产协议缺口；删除后 E2E 负责验证组件与 DOM |

## 当前调用链

组件 → output → `MessageComponent` handler → `ShellService` → canonical command；V2 `ProjectionBusService → ProjectionStore → SurfaceRuntime` 是唯一业务 Frame 消费路径。

当前页面从 facade 读取九个 VM，证据：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/message.component.ts:110`。九 Surface attach 由 `MessageSurfaceFacade` 统一管理，证据：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts:35`。

## 所有权

- Helix 拥有消息排序、权限、未读、回执、身份收敛和 action enablement。
- adapter 只把完整 VM 映射为组件输入，复杂度为每 mutation 一次 `Map` 查找，平均 `O(m)` 时间、`O(m)` 暂存空间。
- 组件只拥有输入焦点、抽屉开关、滚动锚点等短生命周期 UI 状态。
