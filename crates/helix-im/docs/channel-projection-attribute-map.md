# 左侧会话列表 Channel Projection 属性映射（历史快照）

> **状态：`RETIRED_HISTORICAL`。** 本文描述旧 `im:interaction:frame` / `DialogListFrameState`
> 投影链，保留作迁移对照。当前 channel/dialog 事件与查询合同以 Spec 047、
> `crates/helix-im/src/event/` 和 `tests/message_v3/` 为准；不得把本文旧 frame schema 当作现行事件。

## 目的与边界

`im:interaction:frame` / `message.left-dialog-list` 是左侧会话列表唯一的生产消费合同。Helix 将本地
`channel` 行、成员事实和写后累计 DialogPatch 组装为 render-ready ViewModel；客户端只校验
`scope + revision`，按 `orderedChannelIds` 原样渲染。它不得解析 `lastPost`、叠加未读、选择头像成员或重新排序。

`preview` 是最后一条展示摘要；合同不传递、也不要求客户端消费 `lastPost` 原始对象。`topicCount`
只表示可展示的话题数量，详情仍按需查询。

## Frame 外层

| 属性 | 功能 | Helix 权威来源 | 客户端动作 |
|---|---|---|---|
| `schemaVersion` | 合同版本闸门 | `dialog_frame::SCHEMA_VERSION` | 不匹配即拒绝并请求 snapshot |
| `surface` | 目标 UI 面 | 固定 `message.left-dialog-list` | 只交给左侧会话列表 reducer |
| `frameId` | 可观测的发布标识 | Helix revision | trace / 去重辅助，不参与业务推导 |
| `scope.viewerUserId` | 当前观看者身份 | runtime identity | 防止跨账户列表污染 |
| `scope.teamId` | 当前团队隔离键 | runtime identity | 防止跨团队列表污染 |
| `mode` | `snapshot` 或 `patch` | Helix 发布路径 | 选择原子替换或原子属性 patch |
| `baseRevision` / `revision` | 因果与一致性闸门 | `DialogListFrameState` | 仅在 base 精确相等时应用 patch |
| `causationId` | 用户操作确认关联 | 写命令可携带 | 只确认本地 pending UI，不推导行状态 |

## 行属性

| 属性 | UI 功能 | 组装来源 | 禁止的客户端业务逻辑 |
|---|---|---|---|
| `channelId` | keyed row、打开会话 | `channel.id/channel_id` | 生成或替换 ID |
| `title` | 群名/单聊名/公司标签展示 | `displayName/display_name/name` 的 Helix 选择 | 从 raw 字段回退拼接 |
| `channelKind` | 公司、群、单聊、话题的视觉分型 | `channelKind/channelType` 归一 | 通过成员数猜类型 |
| `memberCount` | 成员数量标签 | 持久化 channel 行 | 对成员数组计数 |
| `avatar.kind` | 头像槽位样式 | Helix 根据类型、已知成员和图片事实选择 | 判断群/单聊或选择成员 |
| `avatar.userIds` | `faceUri(userId)` 等图片解析输入 | Helix 先去重；单聊裁为对端一人，群聊拼图最多四人 | 过滤、切片、重排、补成员 |
| `avatar.imageRef` | 自定义群头像引用 | channel 图片事实 | 拼 URL 或判空回退策略 |
| `preview` | 最后一条消息摘要 | `lastMessage` / 已解析 `lastPost` / 写后 post 投影 | 解析 post JSON 或制作摘要 |
| `preview.kind/system/recalled/urgent` | 系统、撤回、加急的图标和文字槽位 | post 类型与加急事实 | 根据文字或 props 猜状态 |
| `preview.senderUserId` | 最后发送者名称 | post sender | 从消息正文推断发送者 |
| `preview.sentAt` | 预览时间 | post/channel 事件时间 | 使用本机时间兜底排序 |
| `badges.unreadCount` | 未读数量 | channel 累计值 / 写后 `dialogPatch` 绝对值 | `++`、`--` 或按 post 计数 |
| `badges.mentionCount` | 艾特数量 | channel 累计值 / 写后绝对值 | 自己扫描 mentions |
| `badges.pendingUrgentCount` | 待确认加急数量 | channel 累计值 / 写后绝对值 | 遍历 expedite/urgent 列表 |
| `state.pinned` | 置顶视觉与排序 | `isTop/is_top` | 将当前会话固定或重新分组 |
| `state.muted` | 静音标识 | channel 通知设置 | 读取 raw settings 或猜测 |
| `state.online` | 在线点 | 已知在线事实 | 用消息活跃时间猜在线 |
| `state.closed/left` | 不可用/已退出状态 | channel 生命周期事实 | 从删帖、成员数推断 |
| `activityAt` | Helix 定序输入 | last post / activity / create 时间 | 客户端排序或 active-row 例外 |
| `topicCount` | 可展示话题数量 | Helix 话题聚合 | 拉全量话题详情或本地计数 |

`avatar.imageRef` 存在时代表 Helix 已选中的自定义会话图片；本端直接渲染该引用。不存在时，
本端才逐个把 `avatar.userIds` 传给平台的 `faceUri(userId)` 资源解析器。两种路径都不改变
Helix 给定的身份集合或顺序。

## Patch 操作

| 操作 | 语义 | 原子性要求 |
|---|---|---|
| `upsertRow` | 只传本次改变的行属性；新行可带完整 `row` | 先校验全部 operation，再一次发布 |
| `removeRow` | 会话已不属于当前 scope | 与排序数组同步删除 |
| `moveRow` | Helix 已得出新的精确位置 | 客户端只移动，不比较 `activityAt` |

任一 schema、scope、identity、order 或 revision 校验失败时，客户端不改变当前可见列表，并合并触发一次
`im_query_dialog_list` snapshot 请求。这保证丢包、重复帧和乱序帧不会产生局部混合状态。
Helix 的可提交 replay 样例位于 `tests/fixtures/dialog_list_frame_v1.json`，固定覆盖完整快照、
属性 patch（含 `false`、`0` 与 clear）以及 revision 断档恢复语义。

## 发布路径

| 领域来源 | 入口 | 产生的列表变化 |
|---|---|---|
| 初始查询、重连、显式 resync | `CorrelationContext::DialogListQuery` | 完整 `snapshot`；只发布 `message.left-dialog-list` frame，不保留 raw 查询投影回退。 |
| 可见 post 写后累计值 | `DialogPatchByPost`、`PersistDialogPatch`、`TooLongReloadPersist` | 一次属性 patch：`preview`、三类 badge、`activityAt` 和必要的 `moveRow`。 |
| 建群、频道资料/成员更新 | `channel_created`、`update_channel`、`channel_member_update`、settings reply | 新行或只含标题、类型、成员、头像、状态、活动时间、话题数的 patch；`update_channel` 同时携带成员绝对 badge/preview 时合并为同一 revision。 |
| 关闭或退出频道 | `channel_close`、`OutboundLeave` reply | 只更新 `state.closed` 或 `state.left`，保持已有行身份与顺序由 Helix 裁决。 |

上述输入都先写入/读取 Helix 权威事实，再由 `DialogListFrameState` 递增 revision 和生成 operation；
不会把 raw WebSocket 或 storage 行交给客户端做二次业务计算。
