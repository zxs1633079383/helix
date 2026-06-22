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

## 1. 投影出口全集（21 工厂 → 20 distinct channel）

| 工厂 fn（src 位置） | channel | data 键集（冻结）| 触发路径 |
|---|---|---|---|
| `to_effect::emit_post_received` | `im:post:received` | `channel_id` `event_seq` `msg_id` | WS `post` echo / sync type1 可见 |
| `to_effect::emit_post_sending` | `im:post:sending` | `channel_id` `temporary_id` | 发送乐观上屏 |
| `to_effect::emit_sync_too_long` | `im:sync:too_long` | `channelId` `resetTo` | sync kind=too_long |
| `to_effect::emit_connection_established` | `im:connection:established` | `connectionId` | hello 握手完成 |
| `to_effect::emit_channels_loaded` | `im:channels:loaded` | `items` | hello 冷启动补齐 |
| `to_effect::emit_channel_increment` | `im:channel:increment` | `channel_id` `increment` | WS `increment_channel` |
| `to_effect::emit_channel_update` | `im:channel:update` | `channel_id` | WS `increment_channel_end` 批次结束 |
| `to_effect::emit_post_read` | `im:post:read` | `channel_id` `event_seq` `msg_id` | 在线 WS `post_read`（type6）|
| `to_effect::emit_channel_read_echo` | `im:channel:read_echo` | `channel_id` `event_seq` `msg_id` | 离线 sync 应用 type6 read |
| `to_effect::emit_post_updated` | `im:post:updated` | `channel_id` `event_seq` `msg_id` | sync type2 编辑（可见）|
| `to_effect::emit_post_batch_updated` | `im:post:batch-updated` | `channel_id` `posts` | WS `posts_update` 批量 |
| `to_effect::emit_post_deleted` | `im:post:deleted` | `channel_id` `event_seq` `msg_id` | sync type3 撤回 |
| `to_effect::emit_channel_update_by_post` | `im:channel:update-by-post` | `channel_id` `event_seq` `msg_id` | sync type1 可见（离线未读 +N）|
| `to_effect_s1::emit_channel_created` | `im:channel:created` | `channel_id` `channel` | WS `channel_created` |
| `to_effect_s1::emit_channel_closed` | `im:channel:closed` | `channelId` `deleteAt` | WS `channel_close` |
| `to_effect_s1::emit_channel_member_updated` | `im:channel:member-updated` | `channel_id` `channel` | WS `channel_member_update` |
| `to_effect_s1::emit_member_nickname` | `im:channel:memberNickname` | `channelId` `userId` `nickName` | WS `update_channel_member_nickName` |
| `to_effect_s1::emit_schedule_created` | `im:channel:schedule-created` | `channelId` `hasSchedulePost` | WS `post_schedule_created` |
| `to_effect_s1::emit_schedule_canceled` | `im:channel:schedule-canceled` | `channelId` `hasSchedulePost` | WS `post_schedule_canceled` |
| `query::emit_message_query_result` | `im:messages:query_result` | `channel_id` `messages` | invoke `im_query_messages_by_channel` |
| `query::emit_dialogs_cleared` | `im:channels:loaded`（第二形态）| `reason` | invoke `im_delete_all_dialogs` |

> **`im:channels:loaded` 双形态**：hello 冷启动路径产 `{items:[]}`（`emit_channels_loaded`），
> delete-all 路径产 `{reason:"delete_all_dialogs"}`（`emit_dialogs_cleared`）——同 channel 名两套
> 触发语义，前端按 `data` 形态区分（有 `items` = 列表刷新信号 / 有 `reason` = 清库刷新信号）。
> 这不是字段漂移，是**两个独立工厂复用同一 channel 名**——测试逐工厂冻结，各断各的键集。

---

## 2. 信号类 vs 投影类（前端如何消费，回填闸门关键）

helix emit 分两类，决定前端「拿到事件后做什么」：

### 信号类（携 `channel_id` + `msg_id` + `event_seq`，前端按 id 回读本地 DB）

`im:post:received` / `im:post:read` / `im:channel:read_echo` / `im:post:updated` / `im:post:deleted`
/ `im:channel:update-by-post` / `im:channel:update`。

这些事件**不携完整 DialogItem/Message 行**——helix 已**先落库再 emit**（gate peek→persist→commit），
权威行已写进 SQLite `message`/`channel` 表（列对齐见 §4）。前端拿 `msg_id`/`channel_id` 去本地读
（经 `im_query_messages_by_channel` 或 dialogList 内存）渲染。**这是「纯渲染」的正解**：字段权威在 DB 行，
emit 只是「哪条变了」的信号。前端**不需要**从 emit payload 补算任何 DialogItem 字段。

> ⚠️ **C2 证伪点**：信号类只需 `channel_id`/`event_seq`/`msg_id` 三字段齐即可定位本地行。若未来有人
> 往信号事件里塞「半份 DialogItem」（如只给 unreadCount 不给 lastPost），前端就会**用半份覆盖整份** →
> 渲染破功。故信号类冻结集**严格 = 三字段**，多一个字段都视为漂移（测试 fail），逼字段走 DB 列而非 emit。

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
im:post:received        → POST handler（按 msg_id 回读本地行渲染）
im:channel:update       → UPDATE_CHANNEL handler（按 channel_id 回读 dialogList 行刷新 badge）
im:messages:query_result→ 投影 handler（直接渲染 messages 数组，{items:[]} 解包）
im:sync:too_long        → onRustTooLong（按 channelId 清内存 buffer + cursor=resetTo-1）
...（其余 16 channel 同理）
```

**任何一条 channel 的 data 键集若与本表不符 → 前端对应 handler 拿不到期望字段 → 渲染破功**。
`tests/projection_schema_test.rs` 把这条「不符即破功」断言成「不符即测试红」。

---

## 6. 命名陷阱词典（cutover 必读，踩一个就联调翻车）

| 陷阱 | 真相 | 出处 |
|---|---|---|
| **`unReadPostId` 大写 R** | 前端代码两套并存：`unReadPostId`（大写R）11 处 + `unreadPostId`（小写r）85 处；DialogItem interface **声明 `unreadPostId`**（小写）。helix patch.rs 写 `unreadPostId`（小写，对齐声明）。**消费侧若读大写R 分支会拿不到值** | `type.d.ts:45` / patch.rs:32 / 前端 grep |
| **`messageType` SYSTEN 拼写** | 系统消息类型枚举有 `SYSTEM`（47 处正确）与 `SYSTEN`（2 处拼写错，漏 M→N）并存于前端；helix 透传 wire `type`/`messageType` 不改写，**SYSTEN 拼写错误保真透传**（改了反而对不上现网那 2 处） | 前端 grep `SYSTE[NM]` |
| **`createAt` 非 `createdAt`/`created_at`** | wire/前端字段是 `createAt`（camelCase，漏 d→`creat`+`At`）；DB 列是 `created_at`（含 d）。patch.rs INT_MAP 显式映射 `("created_at","createAt")`——**helix emit/wire 侧用 `createAt`，DB 列用 `created_at`**，两边都对，别统一 | `type.d.ts:7,130` / patch.rs:45 |
| **`segmentId` 恒空串** | bitmap segment 方案已下线（ADR-013），`segmentId` 字段保留仅为兼容 Rust serde，值**始终空字符串**——禁碰 bitmap/segment（任务铁律） | `type.d.ts:144` |
| **`channelId` vs `channel_id`** | helix emit 信号类用 `channel_id`（snake，自造信号字段）；透传 wire 类用 `channelId`（camel，对齐现网 emit payload）——见 §0/§1 表，逐 channel 不同，不可假设统一 | §1 表 |

---

## 7. 维护铁律

- **本文档与 `tests/projection_schema_test.rs` 是镜像**：改任一 emit 工厂的 `data` 键集，必须**同步**改
  本文档冻结表 + 测试断言；测试红 = 漂移未同步，**禁直接改测试迁就代码**（先确认是有意契约变更）。
- **新增投影 emit 工厂** → 本文档 §1 加一行 + 测试加一个 case（覆盖新 channel 的键集断言）。
- 冻结表行数不变量：§1 表行数 == helix-im `pub fn emit_` 工厂数（`grep -c 'pub fn emit_'` 三文件求和）。
