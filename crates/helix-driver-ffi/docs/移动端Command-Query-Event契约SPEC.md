# 移动端命令 / 查询 / 事件契约规格

> 目标：冻结 Rust 内核向移动端 QuickJS 公开的命令 / 查询 / 事件契约。
> 本文按 LoopForge 的 `docs/ui-指令映射全景.md` 提炼移动端可达 UI 面，结合
> `移动端QuickJS接入最终需求与验收标准.md`、总流程图、发送消息链路图，作为
> `helix-driver-ffi` 后续 Rust FFI 契约 / 能力集 / ABI 验收的输入文档。
> C++ 桥接、QuickJS 注册代码、移动端编译产物和 JS 冒烟不落在 helix 仓，应在移动端集成项目中实现。

## 1. 结论

移动端公开面不是 Rust 内部所有函数，也不是每个业务用例一个 C ABI 符号。

稳定原生边界只保留少量 C ABI：

- 创建 / 销毁 handle
- 设置 / 移除 header
- 命令入口：`command(name, json)`
- 查询入口：`query(name, json)`
- 释放查询结果
- 版本 / 能力集
- 领域事件批回调

业务用例全部通过 `name + JSON payload` 进入 Rust 内核。QuickJS 可以在 JS 层包出
`im.send()`、`im.loadReplies()` 这样的移动端方法，但原生层不随业务膨胀。

Flutter 只负责 UI 渲染和交互采集；QuickJS 只做轻量编排；IM 业务规则、线协议请求体
成形、身份补齐、对账、状态收敛和投影整形都在 `helix-core / helix-im`。

## 2. 真源和边界

本文输入真源：

- LoopForge UI 面：`/System/Volumes/Data/workspace/rust/loopforge-tauri-im/docs/ui-指令映射全景.md`
- FFI 需求：`crates/helix-driver-ffi/docs/移动端QuickJS接入最终需求与验收标准.md`
- 总流程：`crates/helix-driver-ffi/docs/移动端QuickJS接入总流程图.mmd`
- 发送链路：`crates/helix-driver-ffi/docs/发送消息完整链路流程图.mmd`
- 投影字段：`crates/helix-im/docs/projection-schema.md`

本文覆盖：

- 7 屏 UI 已有入口对应的命令 / 查询
- 内核自驱但移动端必须消费的事件
- 请求-响应读族的 `im:read:result`
- 移动端集成项目中的 QuickJS / C++ 桥接必须暴露的生命周期、错误和验收点

仓库边界：

- helix 仓负责：Rust FFI ABI、`helix.h` / `helix.hpp`、native library 构建、capabilities、command / query / event 契约、Rust / FFI 层验收。
- 移动端集成项目负责：C++ `CoreBridge` 类、QuickJS 原生类注册、JS 包装层、Android / iOS 平台构建脚本、C++ / QuickJS 编译产物、真机 smoke。
- helix 仓不得继续新增移动端 C++ bridge 源码、QuickJS wrapper 源码、QuickJS demo 编译产物；这里只保留契约和低层 ABI 验证。

本文不覆盖：

- bot / agent 管理类端点
- Java 归属的搜索类端点
- trace / 跨仓 demo / 废弃端点
- `im_l2_*` 调试专用命令
- Rust 内部辅助函数 / repository / parser / storage API

## 3. CoreBridge 接口

QuickJS 层必须注册一个等价于 `CoreBridge` 的类。

```ts
interface CoreBridge {
  call(name: PublicCommandName, payload?: JsonObject): Promise<Accepted>;
  query(name: PublicQueryName, payload?: JsonObject): Promise<JsonValue>;
  onEvents(listener: (batch: BusEnvelope[]) => void): () => void;
  version(): VersionInfo;
  capabilities(): CapabilitySet;
  dispose(): void;
}

type Accepted = { ok: true; acceptedAtMs?: number };
type JsonObject = Record<string, unknown>;
type JsonValue = null | boolean | number | string | JsonValue[] | JsonObject;
```

语义：

- `call()` 只表示命令已进入 Rust 内核，不表示 Go Server 已完成业务处理。
- 写族业务结果必须通过事件批回流，例如 `im:post:received`、`im:post:updated`。
- `query()` 表示 JS 需要一个可关联结果；底层可以是 FFI 查询，也可以是命令 + `reqId` +
  `im:read:result` / render-ready 事件的桥接等待。
- `onEvents()` 接收 `im:__bus__` 信封批；C++ 必须在回调内立即 memcpy。
- `dispose()` 后再调用任何方法必须返回稳定错误，不能崩溃。

## 4. 载荷命名规则

移动端 JS 封装层面向 Flutter 暴露 camelCase 字段。Rust 负责按命令转换成 Go 线协议所需形态。

公共规则：

- 载荷不传 cookieId / userId / companyId / token；身份来自 Rust handle 上的 headers/profile。
- 读 / 查询请求若需要关联返回，使用 `reqId`；事件回灌中对应字段是 `req_id`。
- `channelId`、`postId`、`temporaryId`、`memberIds` 等字段按 LoopForge UI 映射命名。
- `messageType` / `msgType` 统一由 Rust 兼容；移动端封装层推荐用 `type`，内部转 `msgType`。
- 命名混用是事实契约：事件信号锚有 snake_case，wire/render 核有 camelCase；JS 不得假设全 camel 或全 snake。

## 5. 公开命令

### 5.1 只接收不等待命令

这些命令使用 `CoreBridge.call()`。返回值只表示已接收，业务完成靠事件。

| 名称 | 载荷 | 主要事件 | UI / UC |
|---|---|---|---|
| `im_send_message` | `{channelId, text, temporaryId, type?}` | `im:post:sending` → `im:post:received`；立即失败为 `im:post:send-failed`；无 echo 超时后本地 `send_status='unsend'` | 发送/文档/重发，UC-1.1/1.2/1.4 |
| `im_relay_messages` | `{posts, channelIds}` | `im:post:received` | 转发，UC-1.7 |
| `im_revoke` | `{postId}` | `im:post:batch-updated` / `im:post:deleted` | 撤回，UC-1.5 |
| `im_template_received` | `{postId}` | `im:post:updated` | 模板已收到，UC-3.3 |
| `im_send_quick_reply` | `{postId, emoji}` | `im:post:updated` | 快捷回复，UC-1.8 |
| `im_urgent_post` | `{channelId, postId, targetIds, message?}` | `im:post:updated` | 加急，UC-1.9 |
| `im_urgent_confirm` | `{postId, channelId}` | `im:post:updated` | 加急确认，UC-1.9 |
| `im_create_schedule` | `{channelId, message, schedulePostAt, temporaryId?}` | `im:channel:schedule-created` | 定时消息，UC-1.10 |
| `im_cancel_schedule` | `{channelId}` | `im:channel:schedule-canceled` | 取消定时，UC-1.10 |
| `im_mark_read` | `{postId, channelId}` | `im:post:read` | 单条已读，UC-3.2 |
| `im_read_channel` | `{channelId}` | `im:post:read` / `im:channel:read_echo` | 会话已读，UC-3.1 |
| `im_create_channel` | `{displayName, memberIds}` | `im:channel:created` | 建群，UC-5.1 |
| `im_make_topic` | `{rootId, postId, displayName, memberIds}` | `im:channel:created` | 消息转话题，UC-5.2 |
| `im_channel_close` | `{channelId}` | `im:channel:closed` | 关闭/退出群，UC-5.3 |
| `im_channel_change_display_name` | `{channelId, displayName}` | `im:channel:update` | 改群名，UC-5.4 |
| `im_channel_change_notice` | `{channelId, noticeText}` | `im:channel:update` | 改公告，UC-5.4 |
| `im_channel_change_top` | `{channelId, top}` | `im:channel:update` | 频道置顶，UC-5.5 |
| `im_post_pin` | `{channelId, postId}` | `im:post:updated` | 消息置顶，UC-5.5b |
| `im_channel_member_change` | `{channelId, joinUserIds?, leaveUserIds?}` | `im:channel:member-updated` + `im:channel:members` | 拉/踢人，UC-6.1 |
| `im_channel_set_manger` | `{channelId, userId, set}` | `im:channel:member-updated` / `im:channel:members` | 设/撤管理员，UC-6.2 |
| `im_update_member_nickname` | `{channelId, userId?, nickname}` | `im:channel:memberNickname` | 改群昵称，UC-6.3 |
| `im_announcement_save` | `{channelId, type, message}` | `im:post:updated` | 公告保存，UC-5.6w |
| `im_announcement_read` | `{postId, channelId}` | `im:post:updated` | 公告已读，UC-5.6w |
| `im_announcement_delete` | `{postIds}` | `im:post:updated` | 公告删除，UC-5.6w |
| `im_vote_create` | `{fields}` | `im:post:updated` | 投票发起，UC-8.x |
| `im_vote_do` | `{id, indexes, postId?}` | `im:post:updated` | 投票提交，UC-8.x |
| `im_vote_close` | `{id}` | `im:post:updated` | 投票截止，UC-8.x |
| `im_vote_delete` | `{id}` | `im:post:updated` | 投票删除，UC-8.x |
| `im_average_publish` | `{fields}` | `im:post:updated` | 平均分发布，UC-8.x |
| `im_average_attend` | `{id, score, postId?}` | `im:post:updated` | 平均分打分，UC-8.x |
| `im_average_close` | `{id, postId?}` | `im:post:updated` | 平均分截止，UC-8.x |
| `im_average_delete` | `{id}` | `im:post:updated` | 平均分删除，UC-8.x |
| `im_team_upsert` | `{displayName, memberIds}` | `im:channel:created` | 公司大群，UC-11.1 |
| `im_team_quit` | `{}` | `im:channel:closed` / `im:channel:member-updated` | 退出公司，UC-11.2 |
| `im_sync_channels` | `{}` | `im:net:reconnect_requested` 后跟同步事件 | 按需同步，UC-4.2 |

`im_send` 只保留为旧移动端的入口兼容别名，进入 Rust 后立即归一化为
`im_send_message`。它不是公开能力名，`helix_capabilities.commands` 不得声明该别名；新 QuickJS
代码只能调用 `im_send_message`。

### 5.2 异步请求-结果命令

这些业务读操作同样公开为 `CoreBridge.call()` 命令。需要关联响应时注入 `reqId`，再等待
`im:read:result` 或特定 render-ready 事件。`CoreBridge.query()` / `helix_query` 只用于
`helix_capabilities`、`helix_version` 等同步元数据查询，不能承载 IM 业务查询。

| 名称 | 载荷 | 结果事件 / 返回值 | UI / UC |
|---|---|---|---|
| `im_ready` | `{}` | `boolean` | 就绪探针 |
| `im_query_dialog_list` | `{}` | `im:channels:projection` | 会话列表 / 启动引导 |
| `im_query_messages_by_channel` | `{channelId, limit?}` | `im:messages:query_result` | 首屏消息，UC-2.1 |
| `im_load_older_context` | `{channelId, anchorPostId, anchorCreateAt, limit?}` | `im:messages:older_loaded` | 上拉历史，UC-2.2 |
| `im_get_replies` | `{replyId, reqId?}` | `im:read:result` + `im:channel:replies` | 回复列表，UC-2.4 |
| `im_get_reply_branch` | `{replyFirstLevelId, reqId?}` | `im:read:result` + `im:channel:replies` | 回复分支，UC-2.4 |
| `im_members_by_ids` | `{channelIds, reqId?}` | `im:read:result` + `im:channel:members` | 成员 byIds，UC-6.4 |
| `im_member_snapshot` | `{channelId, startTime, endTime, reqId?}` | `im:read:result` | 成员快照，UC-6.4 |
| `im_channel_query` | `{condition?, pageNumber?, pageSize?, offset?, reqId?}` | `im:read:result` | 条件查频道，UC-5.8 |
| `im_channel_online_status` | `{channelIds, reqId?}` | `im:read:result` | 在线状态，UC-5.7 |
| `im_ensure_channel_loaded` | `{channelId, reqId?}` | `im:read:result` / `im:channel:increment` | 陌生频道兜底，UC-4.5 |
| `im_health` | `{reqId?}` | `im:read:result.body.status` | 健康探针，UC-12.1 |
| `im_modules_get_all` | `{reqId?}` | `im:read:result` | 模块列表，UC-10.3 |
| `im_announcement_accept_list` | `{postId, reqId?}` | `im:read:result` | 公告接受列表，UC-5.6r |
| `im_announcement_list` | `{channelId, postId?, reqId?}` | `im:read:result` | 公告列表，UC-5.6r |
| `im_announcement_detail` | `{postIds, reqId?}` | `im:read:result` | 公告详情，UC-5.6r |
| `im_bookmark_create` | `{channelId, postIds, reqId?}` | `im:read:result` | 书签创建，UC-9.x |
| `im_bookmark_delete` | `{postId, reqId?}` | `im:read:result` | 书签删除，UC-9.x |
| `im_bookmark_load` | `{channelId, pageSize?, pageNumber?, offset?, reqId?}` | `im:read:result` | 书签列表，UC-9.x |
| `im_vote_read` | `{id, reqId?}` | `im:read:result` | 投票详情，UC-8.x |
| `im_average_read` | `{id, reqId?}` | `im:read:result` | 平均分详情，UC-8.x |

> 说明：`im_bookmark_create/delete` 是有副作用的 HTTP，但没有 WS 回声，移动端需要关联响应，
> 因此在 QuickJS 封装层中必须按 `reqId` 关联 `im:read:result`，但入口仍为 `call()`。

### 5.3 GS01-GS03 离线同步 ABI

离线同步验收必须拆分 HTTP、WS、本地持久化、最终投影四个证据面。任一单面成功都不能晋升为
完整同步成功。

| Gate | HTTP 响应 ABI | WS 证据 | 本地提交与最终投影 |
|---|---|---|---|
| `G-S1` | `/channels/load/increment` 返回 `CommonRes.status=SUCCESS,data=null`，只表示异步 worker 已受理 | 同一连接接收零个或多个 `increment_channel.data`，可含 `lastEventSeq` / `needSync`，最后为 `increment_channel_end` | `channel`、`channel_member`、`channel_event_cursor` 同批提交后才能发布 `im:channels:loaded` |
| `G-S2` | `/channel/sync/notify` 返回 `data.entries` 与 `serverTime`；`kind=no_change|events|too_long`，单频道最多 500 个 event，gap 大于 5000 返回 `too_long` | 该 HTTP 请求本身不产生业务 WS；并发 `post` / `posts_update` / `increment_channel_end` 是独立证据 | events/messages 提交后按 `nextSeq` 推进 cursor；`resetTo` 只在恢复事务完成后生效，再发布 `im:channel:update-by-post` |
| `G-S3` | 补偿读取沿用 G-S2 响应 ABI | `pong.data.text=pong`；对账时可含 `gaps[{channelId,fromSeq,maxSeq}]` 与 `hashMismatch` | 连续 events/messages/cursor 提交后才发布 `im:sync:state` |

`messages` 只包含当前调用者可见的正文。HTTP `SUCCESS`、`increment_channel_end`、远端行存在或
普通 `pong` 都不得替代客户端本地事务提交。

## 6. 公开事件

FFI 回调给 C++ 的每个条目必须是 `im:__bus__` 信封：

```json
{
  "channel": "im:post:received",
  "payload": {
    "event": "im:post:received",
    "data": {}
  }
}
```

### 6.1 消息事件

| 频道 | 数据形状 | 必须处理 |
|---|---|---|
| `im:post:sending` | `{channel_id, temporary_id, msgId?, temporaryId?, channelId?, eventSeq?, sendStatus, readBits?, text?, type?}` | 插入发送中乐观行 |
| `im:post:send-failed` | `{channelId, temporaryId, sendStatus, serverId, isSelf, actionCapabilities}` | HTTP/上传立即失败时收敛为可重试失败态 |
| `im:post:received` | 完整 `MessageItemData` | 按 temporaryId / 服务端 id 对账并渲染已发送 |
| `im:post:updated` | 完整 `MessageItemData` | 修补 reactions/urgent/template/pinned/vote/average |
| `im:post:read` | 完整 `MessageItemData` | 更新 readBits |
| `im:channel:read_echo` | 完整 `MessageItemData` | 离线已读回声 |
| `im:post:deleted` | 完整 `MessageItemData` | 标记撤回 |
| `im:post:batch-updated` | `{channel_id, posts}` | 批量撤回/更新 |
| `im:messages:query_result` | `{channel_id, messages}` | 首屏消息渲染 |
| `im:messages:older_loaded` | `{channelId, messages, hasMore}` | 前置追加更早历史 |

移动端必须遵守与 PC/Tauri 一致的发送状态机，不新增移动端专属失败规则：

1. `im_send_message` 被接收后立即产生 `im:post:sending` 乐观行；兼容入口 `im_send` 只做名称归一化。
2. Go WS `post` 回声回来后，内核用 `temporaryId` 对账，产生 `im:post:received`，并将消息收敛为已发送。
3. HTTP 或上传端口立即失败时，内核产生 `im:post:send-failed`；移动端消费该投影进入可重试失败态，不从 JS catch 或错误字符串合成业务状态。
4. 若请求已发出但在发送超时时间内没有收到回声，`PendingSend` 将本地持久化状态标为 `send_status='unsend'`。移动端通过后续查询/重载/投影刷新展示未发送。

`im:post:send-failed` 已由 PC/Tauri 与 FFI 共用的 Helix 内核发出，并冻结在 `projection-schema.md`；`helix_capabilities.events` 必须同步声明，避免移动端按能力注册时丢弃真实事件。

### 6.2 频道 / 会话事件

| 频道 | 数据形状 | 必须处理 |
|---|---|---|
| `im:channels:projection` | `{dialogList}` | 完整频道列表渲染 |
| `im:channels:loaded` | `{items}` 或 `{reason}` | 冷启动/清库信号 |
| `im:channel:increment` | `{channel_id, increment}` | 注册/更新频道增量 |
| `im:channel:update` | `{channel_id}` | 触发 dialogList 刷新 |
| `im:channel:update-by-post` | `{channel_id, event_seq, msg_id}` | 未读 / 徽标信号 |
| `im:channel:created` | `{channel_id, channel}` | 插入或更新频道 |
| `im:channel:closed` | `{channelId, deleteAt}` | 删除/隐藏频道 |
| `im:channel:schedule-created` | `{channelId, hasSchedulePost}` | 定时徽标为 true |
| `im:channel:schedule-canceled` | `{channelId, hasSchedulePost}` | 定时徽标为 false |

### 6.3 成员 / 回复 / 待办事件

| 频道 | 数据形状 | 必须处理 |
|---|---|---|
| `im:channel:member-updated` | `{channel_id, channel}` | 冻结契约面，可用于诊断 |
| `im:channel:memberNickname` | `{channelId, userId, nickName}` | 修补成员昵称 |
| `im:channel:members` | `{channelId, members:[{memberId,nickname,admin}], leaves:[memberId]}` | 可直接渲染的成员列表 |
| `im:channel:replies` | `{reqId, replyIds}` | 可直接渲染的回复抽屉 |
| `im:todo:updated` | `{items:[{id, channel, post, type, canDel, todoId?, todoType?}]}` | 待办面板 |

### 6.4 请求 / 控制事件

| 频道 | 数据形状 | 必须处理 |
|---|---|---|
| `im:read:result` | `{req_id, body}` 或 `{req_id, error}` | 解析查询 Promise |
| `im:connection:established` | `{connectionId}` | 连接诊断/状态 |
| `im:sync:too_long` | `{channelId, resetTo}` | 同步重拉提示 |
| `im:net:reconnect_requested` | `{}` | 驱动观察后重连 |
| `im:net:offline` | 由实现定义 | 断网状态 |
| `im:http:unauthorized` | 由实现定义 | 401 / 重新登录 |

## 7. 能力集

`capabilities()` 至少返回：

```json
{
  "abi": 1,
  "bus": "im:__bus__",
  "commands": ["im_send_message", "im_query_dialog_list"],
  "queries": ["helix_capabilities", "helix_version"],
  "events": ["im:post:received", "im:post:send-failed", "im:post:increment-failed"],
  "features": {
    "pendingSendTimeoutUnsend": true,
    "sendFailedProjection": true,
    "batchEventCallback": true,
    "readResultReqId": true,
    "noPerBusinessAbiSymbol": true
  }
}
```

启动时 QuickJS 封装层必须检查：

- `abi >= 1`
- `bus == "im:__bus__"`
- 本文第 5 节公开的业务操作至少全部存在于 `commands`，或明确返回不支持；`queries` 只允许同步元数据查询
- `commands` 必须包含 `im_send_message` 且不得包含兼容别名 `im_send`
- `events` 必须同时声明运行时真实发布的 `im:post:send-failed` 与 G10 的 `im:post:increment-failed`
- `pendingSendTimeoutUnsend == true` 才允许启用生产未发送/重发 UI

## 8. 错误契约

FFI / C++ / JS 三层都必须保留稳定错误码，不靠字符串做业务判断。

| 错误码 | 场景 |
|---|---|
| `INVALID_HANDLE` | 空 handle / destroy 后调用 |
| `INVALID_UTF8` | name 或 payload 不是合法 UTF-8 |
| `INVALID_JSON` | payload 不是合法 JSON object |
| `UNKNOWN_COMMAND` | 命令名未注册 |
| `UNKNOWN_QUERY` | 查询名未注册 |
| `UNSUPPORTED_CAPABILITY` | 当前原生库不支持某能力 |
| `BACKPRESSURE` | 内核或驱动队列满 |
| `NETWORK` | Go Server / WS / HTTP 网络错误 |
| `STORAGE` | 本地存储错误 |
| `INTERNAL` | 其它内部错误 |

`call()` 拒绝只表示命令未被驱动 / 内核正常接收，或 FFI/网络边界发生同步可报告错误；
不等价于“消息业务发送失败”。消息发送的 UI 可见状态由领域状态机回流：
`im:post:sending` 乐观行、WS `post` 回声对账为已发送、发送超时后持久化为 `unsend`。
移动端不得用 JS catch 或错误字符串自行判定消息 failed。

## 9. 外部移动端 native im.* 封装建议

QuickJS 可在移动端集成项目中注入 native `im.*` API，但所有公开方法必须只做字段转交，不承载 IM 业务规则。
以下代码是外部项目实现参考，不应作为源码落到 helix 仓：

```ts
class MobileIm {
  constructor(private core: CoreBridge) {}

  send(input: { channelId: string; text: string; temporaryId: string }) {
    return this.core.call("im_send_message", {
      channelId: input.channelId,
      text: input.text,
      temporaryId: input.temporaryId,
      type: "TEXT",
    });
  }

  loadReplies(replyId: string) {
    return this.core.call("im_get_replies", { replyId });
  }

  onBus(listener: (events: BusEnvelope[]) => void) {
    return this.core.onEvents(listener);
  }
}
```

禁止：

- JS 自行拼 Go 端点 / 请求体
- JS 自行补 userId/companyId/token
- JS 自行计算 unread/readBits/role/admin/reaction/template/systemNotice
- JS 从 HTTP 错误字符串推断业务状态
- Flutter 直接调用 Go Server

## 10. 验收矩阵

### 10.1 契约冻结

- 生成 `capabilities()`，包含第 5、6 节全部名称。
- helix 仓内 Rust/FFI 契约测试枚举全部命令 / 查询：未知名称返回 `UNKNOWN_*` 或 `UNSUPPORTED_CAPABILITY`，已知名称不返回 unknown。
- C++/QuickJS 冒烟在移动端集成项目中执行；helix 仓只提供契约、头文件 / 原生库和验收说明。

### 10.2 发送消息端到端

必须跑通：

```text
Flutter 点击发送
  -> JS sendMessage
    -> CoreBridge.call("im_send_message", payload)
      -> C++ bridge
        -> helix_command
          -> Rust IM usecase
            -> Go Server
              -> Rust 收敛 ack / WS echo
                -> projection event batch
                  -> C++ callback memcpy
                    -> JS / Flutter 更新 UI
```

验收事件：

- `im:post:sending` 出现，temporaryId 命中
- 成功路径出现 `im:post:received`，sendStatus 变已发送
- 无 echo 超时路径将本地持久化消息标为 `send_status='unsend'`；移动端通过查询/重载/投影刷新看到未发送状态
- C++ 回调不保存借用指针，必须 memcpy 批数据

### 10.3 查询端到端

至少覆盖：

- `im_ready` 返回 true
- `im_query_dialog_list` 收到 `im:channels:projection`
- `im_query_messages_by_channel` 收到 `im:messages:query_result`
- `im_get_replies` 收到同一 reqId 的 `im:read:result` 和 `im:channel:replies`
- `im_health` 收到 `{req_id, body.status}`

### 10.4 内存 / 生命周期

- create/destroy 1000 次无泄漏趋势
- destroy 后 call/query 返回 `INVALID_HANDLE`
- 查询结果必须由指定 free 函数释放
- 回调借用指针生命周期只在回调内有效
- Rust panic 不跨 `extern "C"`
- 非法 UTF-8 / 非法 JSON / 未知名称均返回稳定错误

### 10.5 性能

至少记录：

- JS 调用到 Rust 命令接收的平均/P95/P99
- 事件批回调到 JS 监听器的平均/P95/P99
- 连续 N 次命令 / 查询后的内存增长
- 高频事件批下队列无无界增长

## 11. 实现分期建议

### P0：契约冻结

- 在 `helix-driver-ffi` 增加能力集静态表。
- 对第 5、6 节名称做快照测试。
- 固化现有发送契约：`im:post:sending` 乐观行、`im:post:received` 回声对账、发送超时后 `unsend` 持久化。

### P1：Rust FFI 契约

- `helix_command` 支持移动端公开命令别名 / 载荷归一。
- `helix_query` 只支持 `helix_capabilities` / `helix_version`；业务查询通过 `helix_command` 进入共享异步 query 状态机，结果从批量 DomainEvent 获取。
- Rust/FFI 测试覆盖批回调、查询结果释放、destroy 后错误、能力集。

### P2：外部移动端 C++ 桥接

- 在移动端集成项目创建 `CoreBridge` C++ 类。
- 在移动端集成项目编译 C++ 桥接与 Android/iOS 原生打包。
- 在移动端集成项目冒烟覆盖全部公开命令 / 查询名称。

### P3：外部 QuickJS / Flutter 集成

- 在移动端集成项目注册 JS 类：`call/query/version/capabilities/dispose/onEvents`。
- 在移动端集成项目 JS 冒烟覆盖全部公开封装层。
- Flutter 只绑定 JS API 和事件渲染数据。
- 不引入 Go 端点 / 请求体整形。
- UI 验收用本文第 10 节矩阵作为准入。
