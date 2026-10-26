# G10 Angular 组件与交互合同

## 当前组件

`MessageSurfaceRuntimeViewComponent`、`MessageComponent`

## 当前入口

- 用户/系统入口：切换频道；上拉更早历史
- 入口模式：Surface attach/resync + direct query
- 调用链：`activateChannel/loadOlder` -> `im_query_messages_by_channel` / `im_load_older_context`

## 消费边界

- Angular 只接受 `schemaVersion=2`。
- `ProjectionBusService -> ProjectionStore -> SurfaceRuntime` 先 stage 全帧，再原子 publish。
- 组件只读取 typed Surface VM，不从 raw HTTP、WS、props 或 command receipt 重建业务事实。
- route、draft、overlay 可留在 shell；服务端事实、权限、排序、revision 和 ActionBinding 属于 Helix。

## 验证

- 专属测试：`src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts`
- 公共 codec/runtime：`surface-contract.codec.test.ts`、`surface-runtime.service.test.ts`
