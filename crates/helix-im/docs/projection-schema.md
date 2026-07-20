# projection-schema.md — `im:__bus__` 投影 payload 字段集冻结（C2 真源）

> **本文件是 cutover 契约真源**：helix 经单总线 `im:__bus__` emit 出的**每个业务 channel 的
> `payload.data` 字段集**冻结于此。配套强制测试 `tests/projection_schema_test.rs` 是本文档的
> **可执行镜像**——构造各 emit 工厂输入 → 断言 emit 出的 `data` 键集 == 本文档冻结集（缺失/多余即 fail）。
>
> **为什么冻结（纯渲染证伪标准，接管架构 01 文档 §2 C2）**：前端退化为纯渲染——不再算
> unread/lastPost/readBits/cursor、不维护第二套 sync 状态机。前端只 `listen("im:__bus__")` →
> `CoreEventManager.dispatch` → 渲染。**若某 channel 的 payload 字段不齐，前端就必须补算 = 纯渲染破功**。
> 本文档把「字段齐不齐」从口头约定降级为**机器断言**：字段集漂移 → 测试红。
>
> 真源对齐：字段名/形态逐一锚现网 cses-client `src-tauri/src/features/im/handlers/*` emit 点
> （行号标在各工厂 doc 注释）；前端消费契约 = `src/pages/message-v3/type.d.ts`
> （`MessageDialog.DialogItem` + `Message.MessageItemData`）。

---

## 0. 信封模型（一次性说明，不重复）

helix-im 的每个 emit 工厂产出 `Effect::Emit { event: DomainEventBytes }`，载荷是 JSON
`{"event": "im:xxx", "data": {...}}`。driver-native `event_sink.rs::to_bus_envelope` 把它转成
`im:__bus__` 信封：

```
DomainEventBytes  {"event":"im:post:received","data":{...}}
        │  to_bus_envelope:  channel := payload.event ; payload := 整条 {event,data}
        ▼
im:__bus__ envelope  {"channel":"im:post:received","payload":{"event":"im:post:received","data":{...}}}
        │  guard_bus_envelope:  ① channel 必 im: 开头且非 im:__（防自环）
        │                       ② payload.data 若为 JSON 数组 → 包成 {items:[...]}（M4 单一包装点）
        ▼
app.emit("im:__bus__", envelope)  →  前端 ImEventBusDispatcher → CoreEventManager.dispatch
```

**本文档冻结的是 `data` 对象的键集**（即工厂直接产出、未经 driver guard 包装前的形态）。
`{items:[]}` 数组包装是 driver 出口横切，不在 helix-im 冻结范围（helix-im 产数组 `data`，driver 包）。

**字段命名约定（陷阱预警，§6 详列）**：helix 的 `data` 键**混用** snake_case（信号类 channel_id/
event_seq/msg_id，对齐 EventEnvelope 内部字段）与 camelCase（透传现网 wire 形态的字段 channelId/
deleteAt/resetTo/...）。**这是有意保真**——透传帧/对齐现网 emit payload 的字段保持 wire camelCase，
helix 自造的信号字段用 snake_case。前端 dispatcher 按本表逐 channel 解，不可假设全 camel 或全 snake。

---

## 1. 投影出口全集（24 工厂 → 23 distinct channel）

> **spec06 FixB 标注**：message-row 信号已从「三键瘦信号」改为「**完整可渲染 MessageItemData**」（fat，
> 数据在手零回读，见 §2）。左侧会话列表的写后累计值只进入
> `message.left-dialog-list` 的 `im:interaction:frame`，由一个原子 property patch 覆盖会话行；
> `im:channel:update` 仍是 channel surface 的批次结束瘦信号。**fat 完整集 =
> `message_item_data` 产物**：`channel_id`/`event_seq`/`msg_id`（信号锚 snake）
> + `id`/`msgId`/`eventSeq`/`text`/`createdAt`（与本地消息查询一致的 canonical render-ready aliases）
> + `temporaryId`/`channelId`/`userId`/`type`/`message`/`props`/`createAt`/`updateAt`/`readBits`/`viewers`（现网兼容渲染核）
> + `sendStatus`/`serverId`/`isSelf`/`actionCapabilities`/`reactions`/`reactionCount`/
> `templateReceived`/`templateConfirmedCount`/`urgentTargetCount`/`urgentConfirmedCount`/
> `urgentRequesterId`/`urgentTargetIds`/`urgentConfirmedIds`/`urgentRequiredCount`/`urgentState`/
> `canConfirmUrgent`/`forwardDetail`/
> `expediteMap`/`systemNotice`/`pinned`（**render-ready 终态**·message-v4 纯渲染授权演进）。
>
> **issue #53（S4 首条竖切·C013 纯渲染壳·授权契约演进）**：fat 在渲染核之外补 4 个 render-ready 终态字段，
> 把原本前端壳里做的 shaping（extractReactions / extractTemplateReceived / isSystemNotice / `type||"TEXT"` 兜底）
> 下沉 helix，前端退化为纯绑定（第二北极星）。`sendStatus`=已对账态恒 `"sent"`（入库成功=消息收到·前端收到即取消转圈，
> 不在壳判 echo 来没来）；`reactions`=预解 props.quickReply 的 emoji 串（无→null）；`templateReceived`=props.template.userIds
> 非空 bool；`systemNotice`=type∈NOTICE_TYPES bool；`pinned`=post_pin 下沉 props.pinned（无→null）；`type` 空串容错 `"TEXT"`。**这些字段都从在手 PostFields/props 派生，
> 不触 PostFields 扩字段、不碰 parser 热路径**（守 HX-C005·与「加急 expediteMap 不吐」同理：那个要扩 PostFields，故仍 DB 权威）。
>
> **issue #54（S5 频道列表族·C013 纯渲染壳·授权契约演进）**：低频 `im_query_dialog_list`
> 将 channel scan 转为 `message.left-dialog-list` snapshot；写后 `GuardedBump + Get(channel)` 只产生
> 同一 surface 的 property patch。行内 display、头像、预览、badge 与排序均在 Helix 完成，客户端不接收
> raw dialogList、raw lastPost 或事件 delta。

| 工厂 fn（src 位置） | channel | data 键集（冻结）| 触发路径 |
|---|---|---|---|
| `to_effect::emit_post_received` | `im:post:received` | **fat**（基础消息字段 + `serverId` `isSelf` `actionCapabilities` `reactionCount` `templateConfirmedCount` `urgent*` 双视角字段 `expediteMap` `forwardDetail`，精确全集见测试 `FAT_KEYS`）| WS `post` echo / sync type1 可见 |
| `to_effect::emit_post_sending` | `im:post:sending` | `channel_id` `temporary_id` `msgId` `temporaryId` `channelId` `eventSeq` `sendStatus` `serverId` `isSelf` `actionCapabilities` `readBits` `text` `type`（render-ready 乐观终态行）| 发送乐观上屏 |
| `to_effect::emit_post_send_failed` | `im:post:send-failed` | `channelId` `temporaryId` `sendStatus` `serverId` `isSelf` `actionCapabilities`（`canRetry=true`）| `posts/create` HTTP PortError 立即失败 |
| `render_ready_candidates::emit` | `im:user:candidates` | `reqId` `users[]`（`userId` `username` `nickname` `displayName` `deptId` `deptName` `orgId` `orgName`）| fresh-account 建群候选目录 HTTP 回报 |
| `to_effect::emit_sync_too_long` | `im:sync:too_long` | `channelId` `resetTo` `mode` `dropLocalMessages` | sync kind=too_long / gate buffer overflow（drop local messages + reload） |
| `to_effect::emit_connection_established` | `im:connection:established` | `connectionId` | hello 握手完成 |
| `to_effect::emit_channels_loaded` | `im:channels:loaded` | `items` | hello 冷启动补齐 |
| `to_effect::emit_channel_increment` | `im:channel:increment` | `channel_id` `increment` | WS `increment_channel` |
| `to_effect::emit_channel_update` | `im:channel:update` | `channel_id`（**留瘦**·channel-row）| WS `increment_channel_end` 批次结束 |
| `to_effect_s1::emit_channel_settings_updated` | `im:channel:settings-updated` | `channelId` `settings` `settingVersion` `channelCapabilities` | WS `update_channel` 携群名/简介/导向、服务端 capability 矩阵和当前 viewer role 回声 |
| `to_effect::emit_post_read_with_receipt_revision` | `im:post:read` | **fat**（同 `emit_post_received` 完整集）+ `receiptRevision` | 在线 WS `post_read`（type6）；前端只用不透明版本刷新回执，不观察 bitmap |
| `to_effect::emit_channel_read_echo` | `im:channel:read_echo` | **fat**（同 `emit_post_received` 完整集）| 离线 sync 应用 type6 read |
| `to_effect::emit_post_updated` | `im:post:updated` | **fat**（同 `emit_post_received` 完整集）| sync type2 编辑（可见）|
| `to_effect::emit_post_batch_updated` | `im:post:batch-updated` | `channel_id` `posts` | WS `posts_update` 批量 |
| `to_effect::emit_post_deleted` | `im:post:deleted` | **fat**（同 `emit_post_received` 完整集·撤回 PostFields 稀疏→多空串）| sync type3 撤回 |
| `to_effect_s1::emit_channel_created` | `im:channel:created` | `id` `channelId` `channel_id` `displayName` `group` `channelType` `channelKind` `memberCount` `createdAt` `lastActivityAt` `top` `projectionSource=channel_created` `props` `channel` | WS `channel_created`；新群即时列表直接消费顶层排序事实，不解析嵌套 raw channel |
| `to_effect_s1::emit_channel_closed` | `im:channel:closed` | `id` `channelId` `closed` `deleteAt` | WS `channel_close` |
| `to_effect_s1::emit_channel_member_updated` | `im:channel:member-updated` | `channel_id` `channel` | WS `channel_member_update` |
| `to_effect_s1::emit_member_nickname` | `im:channel:memberNickname` | `channelId` `userId` `nickName` | WS `update_channel_member_nickName` |
| `to_effect_s1::emit_schedule_created` | `im:channel:schedule-created` | `id` `channelId` `hasSchedulePost` `scheduleId` `scheduleMessage` `scheduleMessagePreview` `scheduleSendAt` `scheduleRevision` `scheduleStatus`（同时保留通用 schedule 字段） | WS `post_schedule_created`；全文仅 owner/服务端授权可见，preview 不得替代全文 |
| `to_effect_s1::emit_schedule_canceled` | `im:channel:schedule-canceled` | 同 created，且 `hasSchedulePost=false` `scheduleStatus=canceled` | WS `post_schedule_canceled`；取消为终态，未授权时正文与摘要均为 `null` |
| `query::emit_message_query_result` | `im:messages:query_result` | `channel_id` `messages` | invoke `im_query_messages_by_channel` |
| `query::emit_dialog_list_frame_result` | `im:interaction:frame` | `schemaVersion` `surface` `frameId` `scope` `mode` `baseRevision?` `revision` `causationId?` `snapshot?` `patch?` | invoke `im_query_dialog_list` / 左侧列表写后 patch |
| `query::emit_dialogs_cleared` | `im:channels:loaded`（第二形态）| `reason` | invoke `im_delete_all_dialogs` |

> **`im:interaction:frame` 左侧会话列表合同**：这是 `message.left-dialog-list` 的唯一生产消费面。
> snapshot 带 `rows + orderedChannelIds`，patch 仅带变更属性和 `moveRow`；客户端必须在
> `scope`、`schemaVersion` 与 `baseRevision` 全部匹配时一次性应用，失败只请求新的 snapshot。
> 行内 `preview` 已替代 raw `lastPost`，`avatar.userIds` 是平台 `faceUri(userId)` 的解析输入，
> `topicCount` 只给数量而不夹带话题详情。拒绝的 frame 必须记录结构化原因并请求 snapshot；
> 不存在 raw 列表投影或旧 reducer 回退。

> **`im:channels:loaded` 双形态**：hello 冷启动路径产 `{items:[]}`（`emit_channels_loaded`），
> delete-all 路径产 `{reason:"delete_all_dialogs"}`（`emit_dialogs_cleared`）——同 channel 名两套
> 触发语义，前端按 `data` 形态区分（有 `items` = 列表刷新信号 / 有 `reason` = 清库刷新信号）。
> 这不是字段漂移，是**两个独立工厂复用同一 channel 名**——测试逐工厂冻结，各断各的键集。

### 1.1 连接控制信号（**不计入** §1 RenderNode 投影集 · F1-gap1）

`im_reconnect` 命令（04 文档表 B `IMWS:reconnect`，连接生命周期归 driver）经 `to_effect::emit_reconnect_requested`
产一条**连接控制信号**——它**不是投影**（不携业务渲染数据，`data: {}`），故**不纳入 §1 的 26 个投影工厂 / 25 个 distinct channel**，
与既有 driver 侧横切信号 `im:net:offline`（断网）/ `im:http:unauthorized`（401，`http_cross.rs`）**同族**：
`im:net:*` 控制事件通道，driver/前端**观察**后做连接生命周期动作（reconnect / 重登）。

| 工厂 fn（src 位置） | channel | data 键集（冻结）| 触发路径 | 归属 |
|---|---|---|---|---|
| `to_effect::emit_reconnect_requested` | `im:net:reconnect_requested` | （空 `{}`，无参控制信号）| invoke `im_reconnect` | helix-im 产，driver 观察重连 |

> **为何分开冻结**：§1 RenderNode 投影集是「前端渲染靠的字段契约」（缺字段=补算=纯渲染破功），逐 channel 严格冻结键集。
> 控制信号无渲染字段、走连接生命周期通道——混进投影集会污染「投影=渲染数据」语义；控制信号由独立用例冻结 channel 与空 `data`。

### 1.2 读族回灌通道（**不计入** §1 RenderNode 投影集 · spec06 缺陷A）

读族 outbound 命令（top20/getReplies/bookmark/announcement/user-misc 读/bot 读…）**无 WS 回声、HTTP
响应体本身就是数据**。dispatch 对读命令注册 `CorrelationContext::OutboundReadReply{req_id}`，HTTP
回报时经 `query::emit_read_result` 把**后端响应体原样透传**回灌——它是 **request-response 传输通道**，
**不是投影**（body 由后端定，helix 不冻结字段、不改业务数据，后端权威），故**不纳入 §1 RenderNode 投影集**。
前端 bridge（仿 `helixQueryBridge`）`invoke(命令, {…, req_id})` 后等 `im:read:result{req_id}` resolve → 直接渲染。

| 工厂 fn（src 位置） | channel | data 键集 | 触发路径 | 归属 |
|---|---|---|---|---|
| `query::emit_read_result` | `im:read:result` | `{req_id, body}`（body=后端响应体透传，结构由后端定）| 读族 HTTP Ok 回报 | helix-im 透传，前端 bridge resolve |
| `query::emit_read_error` | `im:read:result` | `{req_id, error}`（诊断字符串，无敏感数据）| 读族 HTTP Err / 信封畸形 | helix-im 产，前端 bridge reject |

> **为何不冻结字段集**：读 = 后端权威透传（后端返回什么吐什么），body 形态随 endpoint 变（top20 返消息数组 /
> bookmark 返书签列表 / status/ids 返状态 map…），逐 endpoint 不同——冻结统一字段集会破坏透传语义。
> `data` **外层**键集恒为 `{req_id, body}`（成功）/ `{req_id, error}`（失败），由 `read_relay_test` 断言。
> 写族保持 fire-and-forget（数据走 WS 回声），**不**走本通道。

### 1.3 上拉翻页投影（**不计入** §1 RenderNode 投影集 · D3 NEED_HELIX_FIX_PULL_REFRESH）

`im_load_older_context` 解上拉到顶的远端兜底翻页（UC-2.2）：多轮 `posts/postContext` 编排（moving anchor +
严格更早过滤 + temporaryId 去重 + pivot 推进，≤8 轮，对齐 legacy `fetch_older_posts_by_context`），凑够
`limit` 条严格更早 / 服务端耗尽 → emit `im:messages:older_loaded`。`messages[]` 是**严格更早 wire Post
升序数组透传**（camelCase `createAt`/`temporaryId`/server `id`，**后端权威不改业务数据**，HX-C005），故
inner Post 形态不冻结（同 §1.2 透传语义）；`data` **外层**键集恒为 `{channelId, messages, hasMore}`。

| 工厂 fn（src 位置） | channel | data 键集 | 触发路径 | 归属 |
|---|---|---|---|---|
| `older_context::emit_older_loaded` | `im:messages:older_loaded` | `{channelId, messages, hasMore}`（messages=严格更早 wire Post 升序数组透传）| invoke `im_load_older_context` → 多轮 postContext HTTP 收尾 | helix-im 编排，前端 prepend |

> **为何不冻结 inner 字段集**：messages 元素 = 后端 postContext 返回的 wire Post 原样透传（同读族），逐字段
> 由后端定。**外层** `{channelId, messages, hasMore}` 由 `tests/p3c_load_older_context_test.rs` 断言。
> `hasMore` 语义：停因「凑够 target / 轮数耗尽」→ `true`（还有更早可拉）；停因「服务端无更早 / pivot 卡住 /
> 空批」→ `false`（到顶）。多轮在 corr_map 续接（每轮单 `Effect::Http`，sans-IO 不阻塞，HX-C001）。

### 1.4 通知/待办装配信号（**不计入** §1 RenderNode 投影集 · UC-10 通知/待办）

`im:todo:updated` 是**内核自驱**（非前端命令发起）的待办列表装配信号：global `increment_channel_end`（hello
收尾）触发 `posts/queryTodoList`，回报后解 `data.channels[].posts[]` 装配成 todo items emit。**外层** `data`
键集恒为 `{items}`（object 包裹，**禁裸数组**——前端 Electron 多参兼容层 `Array.isArray→spread` 会误判，
真源 `channel.rs:346-348`）；每 item = `{id:"{postId}_{messageType}", channel, post, type, canDel}`（channel/post
后端权威透传，inner 不冻结；`id`/`type`/`canDel` 由 helix 按真源 `channel.rs:288-292` 装配）。

| 工厂 fn（src 位置） | channel | data 键集 | 触发路径 | 归属 |
|---|---|---|---|---|
| `todo::emit_todo_updated` | `im:todo:updated` | `{items:[{id, channel, post, type, canDel}]}`（channel/post 透传，id/type/canDel 装配）| hello 收尾 global increment-end → queryTodoList HTTP 回报 | helix-im 装配，前端 `getTodoUpdated$`→`INIT_TODO_LIST_DATA` |

> **为何不冻结 inner 字段集**：item 的 `channel`/`post` = 后端 queryTodoList 返回的 wire 对象原样透传（同 §1.2/1.3）。
> **外层** `{items}` + 每 item 的 `{id, channel, post, type, canDel}` 外层键集由 `tests/todo_assembly_test.rs` +
> `src/todo_tests.rs` 断言。`status≠SUCCESS/200` 或结构缺失 → 空 `items` 仍 emit（前端无害刷新，不挂起）。

### 1.5 成员族 render-ready 渲染通道（**不计入** §1 RenderNode 投影集 · issue #56 S7·C013 纯渲染壳）

`im:channel:members` 是把**成员渲染整形**（`role→admin` 业务规则 + 四源/byIds 解析重组）从渲染壳下沉
helix-im 后**额外**emit 的 render-ready 渲染通道——前端 `applyChannelMembers` 退化为**纯绑定**（按 `memberId`
keyed upsert + `leaves` 删行，零 extract / 零 role 判 / 零四源合并）。**双源**：① WS `channel_member_update`
（拉/踢人）handler 在冻结 `im:channel:member-updated`{channel_id,channel} 之后**额外**emit 本通道
（members=`collect_members` 四源+join 的 render-ready 终态行·leaves=`collect_member_leaves`）；② byIds
成员快照读族回报（`OutboundMembersByIds`）在冻结 `im:read:result`{req_id,body} 之后**额外**逐 channel emit
本通道（解析 `map[channelId][]` 响应体·载族自愈）。`admin` 成品由 `render_ready_members::role_is_admin`
统一判（raw `CREATOR/MANGER/MANAGER/ADMIN` 与归一 `OWNER/ADMIN` → admin·BOSS/MEMBER 否，对齐原壳）。

| 工厂 fn（src 位置） | channel | data 键集 | 触发路径 | 归属 |
|---|---|---|---|---|
| `render_ready_members::emit_channel_members` | `im:channel:members` | `{channelId, members:[{memberId, nickname, admin}], leaves:[memberId]}` | ① WS `channel_member_update`（拉/踢人）② byIds 成员快照 HTTP 回报 | helix-im 整形，前端 `applyChannelMembers` 纯绑定 |

> **为何额外通道而非改既有投影**：冻结的 `im:channel:member-updated`（`{channel_id, channel}`·严格二键）/
> `im:read:result`（`{req_id, body}`）键集**不可加 members 键**（UC-6.1/6.4 ② 契约面「缺/多即 fail」）。
> 故本通道**额外**emit，既有冻结投影原样保留、其 ② 契约面照旧裁定。`im:channel:members` data 外层键集
> （`{channelId, members, leaves}`）+ member inner（`{memberId, nickname, admin}`）由
> `src/render_ready_members_tests.rs` 断言。改群昵称走冻结 `im:channel:memberNickname`（前端 `applyMemberNickname`
> 已是 keyed 单行 upsert·纯绑定·不经本通道——helix 该 WS 帧无全量名册可吐）。
> 内核自驱（无 req_id，非前端 bridge 发起）故独立 `CorrelationContext::TodoQuery`（区别 §1.2 透传 `OutboundReadReply`）。

### 1.6 回复族 render-ready 渲染通道（**不计入** §1 RenderNode 投影集 · issue #57 S8·C013 纯渲染壳·终局切）

`im:channel:replies` 把 `getReplies`/`getReplyBranch` 响应一次扫描为扁平 root + 有序 reply ids +
render-ready nodes。`projectionMode` 由 command 名冻结：`im_get_replies=snapshot`、
`im_get_reply_branch=append`，严禁客户端通过 `reqId` 前缀猜语义。每次请求使用 Helix 单调 correlation
作为 `revision`；同 root 的旧 revision 只保留相关 `im:read:result`，不再 emit render-ready 投影。

Helix 在投影层维护同 root 已见 id 集：snapshot 权威替换，append 跨页去重并保持服务端原顺序。
前端只按显式模式绑定 `messageById` 与 `replyIdsByRoot`。当前 Go 接口支持页码但不返回可恢复 opaque
cursor，因此合同冻结 `cursor:null`；`hasMore` 透传响应字段，缺失时明确为 `false`。

| 工厂 fn（src 位置） | channel | data 键集 | 触发路径 | 归属 |
|---|---|---|---|---|
| `render_ready_replies::emit_channel_replies` | `im:channel:replies` | `{reqId,channelId,rootMessageId,projectionMode,revision,replyIds,nodes,replyCount,cursor,hasMore}` | getReplies/getReplyBranch HTTP 回报（`OutboundReplies` 双 emit）| helix-im 负责乱序抑制、snapshot/append 与去重保序；前端纯绑定 |

> **为何额外通道而非改既有投影**：冻结的 `im:read:result`（`{req_id, body}`·严格二键）键集**不可加 replyIds 键**
> （UC-2.4 ② 契约面「缺/多即 fail」·body verbatim 透传）。故本通道**额外**emit，既有冻结投影原样保留、其 ②
> 契约面照旧裁定 verbatim body。`im:channel:replies` data 外层十键由
> `src/query/render_ready/replies_tests.rs` 与 `tests/projection_schema_test.rs` 精确断言。缺/空 `req_id`
> 降级为 fire-and-forget：HTTP 可执行，但 corr 不注册、不得产生无关联回复投影。**待办族**（§1.4 `im:todo:updated`）S8 同步补 render-ready 终态键
> `todoId`/`todoType`（每 item 在冻结-记录的 `{id, channel, post, type, canDel}` 之上**额外**加·inner 不冻结·
> 外层 `{items}` 键集照旧）→ 前端 `applyTodoUpdated` 退纯绑定（1:1 取 todoId/todoType/canDel·不 rename）。

### 1.7 转发批结果与回执人员 render-ready 通道

| channel | data 键集 | 权威边界 |
|---|---|---|
| `im:posts:batch-result` | `{reqId,batchStatus,acceptedCount,failedCount,deliveryAuthority,targets:[{channelId,acceptanceStatus,deliveryStatus,error}]}` | HTTP 只证明 `accepted/failed`；accepted 的 `deliveryStatus=pending`，最终送达只认逐目标 WS `post` |
| `im:post:readers` | `{reqId,receipts:[{postId,state,snapshotId,snapshotVersion,readerCount,unreadCount,recipientUserIds,readBits,authorExcluded,contractVersion,readerIds,unreadIds,readers:[{userId}],unreaders:[{userId}]}]}` | Helix 按消息发送时 `memberIds` 快照顺序解释 `readBits`；canonical 保持接收者序列，legacy 在线性扫描中剔除作者位，固定输出 `authorExcluded=true`、`contractVersion=2`，非法形态 fail closed |

### 1.8 最近消息查询状态通道（**不计入** §1 RenderNode 投影集 · A2 local-first）

`im_query_messages_by_channel` 的成功结果无论来自 Complete 本地直出，还是 Partial/Miss/Unsupported
远端回源，都统一走冻结投影 `im:messages:query_result{channel_id,messages}`。只有远端 HTTP、信封或
CommonRes 状态失败时，才 emit 独立状态通道 `im:post:increment-failed`；失败不得退化成空消息成功。
该通道只表达本次查询失败，不携带可渲染 message row，因此不计入 §1 RenderNode 投影工厂数量。

| 产出位置 | channel | data 键集（冻结） | 触发路径 | 归属 |
|---|---|---|---|---|
| `query/local_first.rs::emit_message_query_failed` | `im:post:increment-failed` | `{channelId, error}` | `getLatestPost` PortError / HTTP 非 2xx / CommonRes 非 SUCCESS / 信封畸形 | helix-im 查询状态，前端结束 loading 并展示失败 |

消息 `actionCapabilities` 额外冻结五个布尔值：`canUrgentConfirm`、`canMarkRead`、
`canViewReceipt`、`canTemplateConfirm`、`canLocate`。其中 `canLocate` 仅在消息具备 serverId 且未撤回时为真；
其余能力仅由 serverId、viewer 身份、加急 recipients 和模板确认列表生成；
事实不足时为 false。频道 `channelCapabilities` 则由 Go `capabilitiesByRole` 与 Helix 已记忆的当前 viewer
成员角色相交生成，Angular 不接触角色字符串或能力矩阵。

---

## 2. message-row fat vs Dialog patch vs 透传类（前端如何消费 · spec06 FixB 重写）

> **spec06 缺陷B 修复（用户拍板）**：原 §2 把所有 `im:post:*`/`im:channel:*` 信号一律设计成「三键瘦信号 +
> 前端按 id 回读本地 DB」——但 message-row 事件 helix **写完那行时数据就在手**（同一份 `PostFields` 正落库），
> 却只吐个信号逼前端二次查库（N 个渲染组件各查一次 = N 次冗余 DB 读）= **蠢代码**。修复：**message-row 信号
> 一次吐完整可渲染行（fat），UI 直渲零回读**。原「瘦信号防塞半份」的顾虑是针对「**部分**数据」——吐**完整**
> 数据无覆盖问题，顾虑自解。左侧会话列表采用独立的 InteractionFrame reducer：消息投影只确认
> timeline；列表累计值由 Helix 写后 readback 的 surface patch 表达。

helix emit 分三类，决定前端「拿到事件后做什么」：

### message-row fat 类（携完整 MessageItemData 核心字段，前端直接渲染·零回读）

`im:post:received` / `im:post:read` / `im:channel:read_echo` / `im:post:updated` / `im:post:deleted`。

这些事件的渲染对象是**单条消息行**——helix 在 gate apply / sync apply **写完 message 行那一刻，
`PostFields` 就在手**（与 `event_to_storage_op(ev)` 落库用同一份 owned 数据）。`to_effect::message_item_data`
把它**重组**成完整可渲染 `data`（真源 `type.d.ts MessageItemData` ∩ 在手 `PostFields`）：
`channel_id`/`event_seq`/`msg_id`（信号锚 snake，定位/对账/兼容）+ `id`/`msgId`/`eventSeq`/`text`/
`createdAt`（与 `im:messages:query_result` 相同的 canonical aliases）+ `temporaryId`/`channelId`/`userId`/
`type`/`message`/`props`/`createAt`/`updateAt`/`readBits`/`viewers`（现网兼容字段）。**前端直接渲染，零回读**。

> 🔴 **HX-C005 守约（SPEC §2 热路径回读铁律）**：fat 数据**全部来自在手 `PostFields`**——不发 Storage 读
> Effect、不 `serde_json::to_string(整条 wire 对象)`。这是「完整投影」的**唯一正道**：写入时手上的数据直接吐。
> `replyId`/`replyRootId`/`replyFirstLevelId` 已在 parser 单次提取时进入 `PostFields` 并随 message 行落库；
> 首屏/刷新查询把三者整形成 render-ready camel 字段。在线 fat 信号仍不扩键，避免无关事件契约膨胀；
> 回复交互在需要层级锚时走本地消息查询，前端只透传锚，不计算回复树。

> ⚠️ **C2 证伪点（FixB 反转）**：message-row 信号冻结集 = `message_item_data` 完整集（§1 表）。**缺**任一
> 完整字段 → 前端拿不到 = 回退「半份回读」蠢模式 → 测试 fail（`projection_schema_test` 注入缺字段证伪）。
> 与旧契约相反：旧契约「多一字段即漂移」，新契约「**少一字段即漂移**」（完整才是正解）。

### 左侧列表 frame patch 类（可见 post / member 变更）

`im:interaction:frame` 的 `message.left-dialog-list` patch 表达一次完整的列表原子转换：

- `preview`：来自当前可见 post 的最终展示摘要；
- `badges`：写后 `Get(channel)` 或定向 `channel_member` 绝对态的 unread / mention / pending urgent 终值；
- `activityAt` 与 `moveRow`：与同一次 badge/preview 更新共同提交，禁止产生中间位置；
- `baseRevision` / `revision`：客户端仅在精确连续时应用，否则记录拒绝原因并请求 snapshot。

> 🔴 **HX-C005 守约**：该 patch 使用当前 post 的在手 `PostFields` + `PostChannelUpdate` 计算事件语义，
> 再通过同一 Persist 批次里的 `GuardedBump -> Get(channel)` 取得写后累计终值。I/O 仍是 Effect/StorageOp，
> core 不理解 IM 业务列；前端只接收 frame，不做 read-modify-render。

### channel-row thin 类（批次结束单键，历史客户端可低频回读累计 DB 行）

`im:channel:update`（`{channel_id}`）仍是批次结束瘦信号。完整 DialogItem（channel 行，~50 列：
`unreadCount/lastPost/mentionCount/…`）是跨多帧累计结果，不在该事件手上；未迁移客户端可按 batch /
低频 invoke 回读 `dialogList`，而不是每条 post 回读。已挂载 `message.left-dialog-list` frame 的客户端
不消费这条信号驱动列表，也不自行做累计校准。

### 透传/投影类（携完整 wire 数据或 DB 行，前端直接消费）

- `im:channel:increment`（`increment` = 透传 `increment_channel` 帧 data，含 mentionList/urgentPostList）
- `im:channel:created` / `im:channel:member-updated`（`channel` = 透传帧 data 原始 channel 对象）
- `im:post:batch-updated`（`posts` = 透传 `posts_update` 帧的 posts 数组）
- `im:messages:query_result`（`messages` = `Scan message` 的本地 DB 行数组，列名 = §4 DB 列）

透传类**不解释字段**（parser 已在边界零信任解析，此处仅透传），故冻结的是**外层包装键**
（`channel_id`+载荷键），载荷内部字段不在本文档逐一冻结（由现网 wire 契约保真）。

---

## 3. 前端 DialogItem / Message 消费契约（字段权威清单）

> 真源 `src/pages/message-v3/type.d.ts`。helix 经 §4 DB 列 + §2 信号事件保证前端这些字段**全由 Rust 投喂**，
> 前端零补算。下表 = 前端渲染**需要**的字段，对应 helix 哪条路径供给。

### MessageDialog.DialogItem（节选渲染关键字段）

| DialogItem 字段 | helix 供给路径 | 备注 |
|---|---|---|
| `id` `displayName` `type` `teamId` | `channel` 表列（patch.rs 写）→ dialogList 读 | wire camelCase → DB snake（§4）|
| `unreadCount` `mentionCount` `mentionCountRoot` | `channel` 表列 + `im:channel:update*` 信号触发回读 | **禁前端算**：权威在 DB 列 |
| `unreadPostId` | `channel.unread_post_id` 列（patch.rs:32）| ⚠️ 命名陷阱见 §6（前端 `unReadPostId` 大写R 与声明 `unreadPostId` 并存）|
| `lastPost` `lastPostAt` `lastRootPostAt` | `channel` 表列（patch.rs）| `lastPost` 整对象进单列 |
| `hasUrgentPost` `hasSchedulePost` `urgentPostList` | `channel` 表列 + `im:channel:schedule-*` 信号 | bool/json 列 |
| `mentionList` `topCount` `channelIsTop` | `channel` 表列 | json/int/bool |
| `lastEventSeq` | `channel.last_event_seq`（sync cursor，HX-C008）| 后端 d4748d62 引入 |
| `members` `owner` `adminUsers` `boss` | `channel_member` 表（独立成员表，patch 白名单排除）| `im:channel:member-updated` 信号触发回读 |
| `memberCount` `subtopicsLoadedAt` | 本地列（patch 白名单**排除**，Rust 内部维护）| Angular 只读不写 |

### Message.MessageItemData（消息行渲染关键字段）

| 字段 | helix 供给路径 | 备注 |
|---|---|---|
| `temporaryId`（PK）`message` `type` `userId` `channelId` | `message` 表列（send/sync 落库）| `temporaryId` 是热路径 PK（B5 禁 RMW）|
| `readBits` | `message.read_bits` 列（C2 单调覆盖落库）+ `im:post:read`/`im:channel:read_echo` 信号 | **禁前端算 readBits** |
| `createAt` `updateAt` | `message` 表列 | ⚠️ wire camelCase `createAt`（非 `createdAt`/`created_at`，§6）|
| `sendStatus` | `message.send_status`（乐观态 Sending → echo Acked）| send 子机维护 |
| `viewers` `userSnapshot` `replyId` `replyRootId` `threadId` | `message` 表列 | send build 注入 |
| `segmentId` | **已废弃恒空串**（bitmap 下线 ADR-013，保留兼容 serde）| ⚠️ 禁碰 bitmap |

---

## 4. DB 列对齐（patch.rs wire→column，供前端回读理解）

`im:channel:update*` 信号触发前端回读的 `channel` 行，列名由 helix-im `channel_write/patch.rs`
的 wire→column 映射决定（前端 dialogList 读时按 DB 列名拿，driver codec 序列化 rows→JSON）。
**冻结点 = patch.rs 的 STR_MAP/INT_MAP/BOOL_MAP/JSON_MAP**（已有 `channel_write_program_test.rs`
+ `patch.rs` 单测守，本文档不重复断言，仅索引）。关键陷阱列：

| DB 列（snake）| wire 键（camel）| 类型 |
|---|---|---|
| `display_name` | `displayName` | Text |
| `unread_post_id` | `unreadPostId` | Text（⚠️ §6）|
| `last_event_seq` | `lastEventSeq` | Int（sync cursor）|
| `created_at` | `createAt` | Int（⚠️ wire 是 `createAt` 不是 `createdAt`，§6）|
| `unread_count` `mention_count` `mention_count_root` | `unreadCount` `mentionCount` `mentionCountRoot` | Int |
| `is_top` | `channelIsTop` | Bool |
| `has_urgent_post` `has_schedule_post` | `hasUrgentPost` `hasSchedulePost` | Bool |

`message` 表投影列（`im:messages:query_result` 的 `messages` 行）= `SELECT * FROM message`
原始列（debug dump 语义，driver rows→bytes codec）。

---

## 5. 接管前端零改动保证（C2 闭环）

冻结本表后，cutover 时前端 `CoreEventManager.dispatch` 的 channel→handler 映射**逐 channel 命中本表**：

```
im:post:received        → POST handler（**fat 直渲** message_item_data，零回读·spec06 FixB）
im:interaction:frame    → 左侧列表 frame reducer（核验 scope/revision 后原子替换或属性 patch）
im:channel:update       → channel surface handler（不驱动左侧列表）
im:messages:query_result→ 投影 handler（直接渲染 `{channel_id, messages:[...]}`，不走 `{items:[]}` 解包）
im:sync:too_long        → onRustTooLong（按 channelId drop local messages + cursor=resetTo-1 + reload 首屏）
...（其余 16 channel 同理；message-row 信号 fat 直渲 / channel-row 信号 thin 低频回读）
```

**任何一条 channel 的 data 键集若与本表不符 → 前端对应 handler 拿不到期望字段 → 渲染破功**。
`tests/projection_schema_test.rs` 把这条「不符即破功」断言成「不符即测试红」。

---

## 6. 命名陷阱词典（cutover 必读，踩一个就联调翻车）

| 陷阱 | 真相 | 出处 |
|---|---|---|
| **`unReadPostId` 大写 R** | 前端代码两套并存：`unReadPostId`（大写R）11 处 + `unreadPostId`（小写r）85 处；DialogItem interface **声明 `unreadPostId`**（小写）。helix patch.rs 写 `unreadPostId`（小写，对齐声明）。**消费侧若读大写R 分支会拿不到值** | `type.d.ts:45` / patch.rs:32 / 前端 grep |
| **`messageType` SYSTEN 拼写** | 系统消息类型枚举有 `SYSTEM`（47 处正确）与 `SYSTEN`（2 处拼写错，漏 M→N）并存于前端；helix 透传 wire `type`/`messageType` 不改写，**SYSTEN 拼写错误保真透传**（改了反而对不上现网那 2 处） | 前端 grep `SYSTE[NM]` |
| **`createAt` 与 `createdAt`** | 现网 wire/旧前端字段是 `createAt`（漏 d），DB 列是 `created_at`；render-ready 消息投影同时提供 canonical `createdAt`，两者同源。业务写入仍按 `createAt`→`created_at` 映射，纯渲染壳优先绑定 `createdAt` | `type.d.ts:7,130` / patch.rs:45 / `query/render_ready/core.rs` |
| **`segmentId` 恒空串** | bitmap segment 方案已下线（ADR-013），`segmentId` 字段保留仅为兼容 Rust serde，值**始终空字符串**——禁碰 bitmap/segment（任务铁律） | `type.d.ts:144` |
| **`channelId` vs `channel_id`** | helix emit 信号类用 `channel_id`（snake，自造信号字段）；透传 wire 类用 `channelId`（camel，对齐现网 emit payload）——见 §0/§1 表，逐 channel 不同，不可假设统一 | §1 表 |

---

## 7. 维护铁律

- **本文档与 `tests/projection_schema_test.rs` 是镜像**：改任一 emit 工厂的 `data` 键集，必须**同步**改
  本文档冻结表 + 测试断言；测试红 = 漂移未同步，**禁直接改测试迁就代码**（先确认是有意契约变更）。
- **新增投影 emit 工厂** → 本文档 §1 加一行 + 测试加一个 case（覆盖新 channel 的键集断言）。
- 冻结表行数不变量：§1 表行数 == helix-im `pub fn emit_` 工厂数（`grep -c 'pub fn emit_'` 三文件求和）。
