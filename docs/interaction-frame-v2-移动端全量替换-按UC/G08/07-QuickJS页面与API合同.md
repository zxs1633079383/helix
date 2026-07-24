# G08 QuickJS 页面与 API 合同

## 当前入口

- QuickJS binding：`im.posts.createSchedule`。
- command：`im_create_schedule`。
- registrar：`venus/api/ImPostsApi.cpp`。

## V2 页面合同

页面只允许：

1. attach 本 Gate 的 `message.channel-summary`、`message.left-dialog-list`、`message.timeline`。
2. 通过 `im.surface.view({surface,scope})` 读取 ChannelSummaryVm + DialogListVm + 可选 TimelineWindowVm。
3. 展示 `im.surface.pending()` 的本地传输 pending。
4. 将用户动作交给 native host，由 host 使用 Frame 内 opaque ActionBinding 构造完整 envelope。

页面不得直接拼 `im_create_schedule`、订阅 raw business event、查询 SQLite row、合并临时/服务端身份或
重算权限/未读/排序。当前仓没有该 Gate 的真实产品页面文件：`SOURCE_MISSING`；实施前必须登记
页面路径、组件/节点标识和 renderer key。
