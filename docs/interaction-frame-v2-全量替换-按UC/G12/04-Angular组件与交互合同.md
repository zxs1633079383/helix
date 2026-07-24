# G12 Angular 组件与交互合同

## 当前组件

`MessageUiSurfaceThreadDrawerComponent`、`MessageUiReplyDrawerComponent`、`MessageSurfaceRuntimeViewComponent`

## 当前入口

- 用户/系统入口：打开回复抽屉、加载分支或发送回复
- 入口模式：查询与回复发送均为 direct command
- 调用链：`openThread/loadReplyBranch/sendReply` -> `im_get_replies` / `im_get_reply_branch` / `im_send_message`

## 消费边界

- Angular 只接受 `schemaVersion=2`。
- `ProjectionBusService -> ProjectionStore -> SurfaceRuntime` 先 stage 全帧，再原子 publish。
- 组件只读取 typed Surface VM，不从 raw HTTP、WS、props 或 command receipt 重建业务事实。
- route、draft、overlay 可留在 shell；服务端事实、权限、排序、revision 和 ActionBinding 属于 Helix。

## 验证

- 专属测试：`src/pages/message-v4/components/__tests__/message-ui-reply-drawer.component.test.ts`
- 公共 codec/runtime：`surface-contract.codec.test.ts`、`surface-runtime.service.test.ts`
