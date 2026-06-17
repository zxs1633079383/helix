# 客户端 src-tauri Rust 侧 IM 核心梳理

> 范围：`cses-client/src-tauri/src/features/im/`（commands / http / router / message_loop / sync_compensation / event_bus / all_hash / state / domain / handlers / services / repo / tables）+ WS 客户端胶水 `src/features/clients/ws/im_ws_client.rs` + 命令注册 `src/framework/boot.rs`。
>
> 字段级引用 verbatim 摘自源码（标注文件:行）。GitNexus 索引偏旧，本文以活体源码为准。
>
> **核心结论**：服务端 IM-Sync v2 wire 契约的客户端权威实现全在这里。迁移目标后端（cses-im-server）若要复刻 sync/notify、心跳 ping/pong gap 补偿、channel/load/increment 的 `cursors`/`needSync`/`allHash` 协议，必须 byte-exact 对齐本文档的字段名 / 大小写 / 枚举值 / FNV 哈希向量。

---

## 0. 总览数据流（一图胜千言）

```
                     ┌─────────────── Angular (前端) ───────────────┐
   invoke(命令)  ───▶ │ im_sync_channels / im_get_cursor / im_switch_active_channel
                     │ ensure_channel_loaded / im_load_older_messages / im_send_message …
                     └───────────────────────────────────────────────┘
                              │ #[tauri::command]
                              ▼
  WS 入站 ──▶ ImWsHandler.on_message ──▶ mpsc ──▶ message_loop(dispatcher+4worker)
  (im_ws_client)   解析 {action,data,tracing}        │ classify(action) → Realtime/Batch
                                                     ▼
                                              router::route(action) ──▶ handlers/{hello,channel,post,schedule}
                                                     │ 返回 Vec<(event_name, payload)>
                                                     ▼
                                              event_bus::emit_im → app.emit("im:__bus__", {channel,payload,tracing})
                                                     │
                                                     ▼  前端 tauri.service.ts bus dispatcher 按 payload.channel 分发
  心跳 ping(8s) ──▶ build_ping_data{cursors,allHash} ──▶ 服务端 ReconcileCursors
  pong{gaps,hashMismatch} ──▶ compensate_from_pong ──▶ compensate_channel → on_gate_fire → sync/notify
```

---

## 1. `#[tauri::command]` 暴露给前端的命令清单

注册点 `src/framework/boot.rs:248-265`（`tauri::generate_handler![...]`）。bitmap 三命令已下线（boot.rs:252-254 注释）。

| 命令 | 文件:行 | rename_all | 参数（wire key） | 返回 | 服务的前端用例 |
|------|---------|-----------|------------------|------|----------------|
| `im_get_connection_id` | commands.rs:28 | 默认(snake) | 无 | `Option<String>` | webview 刷新后前端 `ImConnectionState._connectionId` 丢失，启动 pull 一次拿回 connectionId（不依赖只触发一次的 `im:connection:established`）|
| `im_send_message` | commands.rs:36 | 默认 | `action:String, data:Value, seq:Option<i64>` | `bool` | 前端发消息走 WS：组 `{action,data,seq}` 经 `WsManager.send_json("im", ...)` 出站 |
| `im_query_channels` | commands.rs:60 | 默认 | `app` | `Vec<Value>` | 会话列表首屏：读本地 SQLite `channel` 全表 + `assemble_channels` 拼成员 |
| `im_query_channel_by_id` | commands.rs:87 | 默认 | `channel_id:String` | `Option<Value>` | 单会话详情：读 `channel` + 拼成员 |
| `im_query_bitmaps_by_channel` | commands.rs:117 | 默认 | `channel_id` | `Vec<Value>` | **已下线**（boot 未注册；bitmap 方案→策略3 状态机）|
| `im_query_bitmap_by_id` | commands.rs:148 | 默认 | `bitmap_id` | `Option<Value>` | **已下线** |
| `im_query_messages_by_channel` | commands.rs:179 | `camelCase` | `channel_id, limit:Option<i64>` | `Vec<Value>` | **debug-only**（`cfg(debug_assertions)`）：「数据链路」面板取最近 N 条原始行；`ORDER BY create_at DESC, temporary_id DESC` |
| `im_delete_all_dialogs` | commands.rs:231 | 默认 | 无 | `bool` | 测试用：清 `channel` + `channel_member` |
| `im_delete_all_bitmaps` | commands.rs:254 | 默认 | 无 | `bool` | 测试用（未注册）|
| `im_switch_active_channel` | commands.rs:277 | `camelCase` | `deactivateId:Option<String>, activateId:String` | `bool` | 切会话：旧 `is_active=false`、新 `is_active=true`+清零 `unread_count`/`unread_post_id`；同步维护心跳 open 集（`heartbeat.mark_closed/mark_open`）。用独立 `tenant_user_db` 写池避开 sync 大批写排队 |
| `ensure_channel_loaded` | commands.rs:331 | `camelCase` | `channel_id:String` | `Value`(channel) | 独立窗口（chat-standalone）本地无数据兜底：HTTP `/api/cses/channel/load/incrementByChannelId` → `process_and_save` 写库 → emit `im:channel:increment` → 返回 channel 立即 `setCurrentDialog`。service 层 singleflight 去重 |
| `im_load_older_messages` | commands.rs:391 | `camelCase` | `channel_id, anchor_post_id, anchor_create_at:i64, limit:Option<i64>` | `Vec<Value>` | 上拉历史远端兜底：本地耗尽时以最旧消息为锚点走 `/api/cses/posts/postContext {postId,before}` 拉更早 N 条 upsert 后升序 prepend。零后端改动 |
| `im_sync_channels` | handlers/sync.rs:55 | 默认(参数 `cursors`) | `cursors: Vec<SyncCursor>` | `SyncResponse` | 增量同步 A3 入口（前端直接驱动 sync）：转调 `domain::sync::sync_channels` → POST `/api/cses/channel/sync/notify` |
| `im_get_cursor` | handlers/sync.rs:86 | `camelCase` | `channel_id:String` | `i64` | 查单 channel 本地 cursor（前端 gap 检测对账）；`im_set_cursor` 已 2026-05-30 下线（cursor 单一真源是 Rust gate）|
| `im_list_cursors` | handlers/sync.rs:109 | 默认 | 无 | `Value` | debug-only：列全部 cursor |

---

## 2. HTTP 出站封装（`http.rs` + `domain/sync.rs`）

统一经 `core::network::http::HttpClient`，实例名 `"im"`（`HttpManager.get("im")`）。`im` 客户端的 `connectionId` header 在 hello 握手时由 `hello::handle` 注入（hello.rs:81-85），后续所有 im HTTP 自动带该头。所有 path 都是 mattermost csesapi base `/api/cses`。

辅助 `post()`（http.rs:9）：`client.request("POST", path, Some(body), TimeoutProfile::Api)` 返回 `resp.body`。

| Rust 函数 | path | body（verbatim） | 用途 / 前端用例 |
|-----------|------|------------------|------------------|
| `get_increment_channels` (http.rs:25) | `/api/cses/channels/load/increment` | `{ "timestamp": <u64>, "cursors": [{"channelId","fromSeq"}] }` | **hello 握手后冷启动全量增量**（机制A 上传侧）。`cursors` 仅根群（root_id 空）的 per-channel max event_seq 快照；服务端据此 stamp 帧顶层 `needSync`（已追平→false→收帧侧跳过 sync 省流）。话题请求**不带** cursors |
| `get_increment_by_channel_id` (http.rs:47) | `/api/cses/channel/load/incrementByChannelId` | `{ "channelId": <str> }` | 独立窗口本地无数据兜底（`ensure_channel_loaded`）。响应形状与 socket `increment_channel` 推送一致，复用 `process_and_save` |
| `get_update_posts` (http.rs:57) | `/api/cses/posts/getUpdatedPosts` | `{ "timeStamp": <u64>, "limit": 20 }` | 拉更新帖子 |
| `query_todo_list` (http.rs:67) | `/api/cses/posts/queryTodoList` | 透传 `{postIds:[...]}` | increment_end 收尾拉待办（@我/紧急 post 详情）|
| `query_latest_posts` (http.rs:72) | `/api/cses/posts/getLatestPost` | `{ "channelId": <str> }` | 拉某 channel 最新 20 条（硬编码 20，timestamp 被服务端忽略，见注释 cses_post.go:1409）|
| `query_post_context` (http.rs:89) | `/api/cses/posts/postContext` | `{ "postId": <anchor>, "before": <i64> }` | 上拉历史唯一「单 channel + 锚点往前」零后端改动接口（`im_load_older_messages` 底层）|
| `sync_channels` (domain/sync.rs:198) | `/api/cses/channel/sync/notify` | `{ "cursors": [SyncCursor] }` | **增量同步核心**：gate fire / 心跳补偿 / 前端 A3 都走这里 |

### 2.1 sync/notify wire 契约（迁移必须 byte-exact 复刻 — verbatim 摘 domain/sync.rs）

**请求** `SyncRequest{ cursors: Vec<SyncCursor> }`（domain/sync.rs:156）。

`SyncCursor`（domain/sync.rs:58-64，`#[serde(rename_all = "camelCase")]`）：
```rust
pub struct SyncCursor { pub channel_id: String, pub from_seq: i64 }
// wire: {"channelId":"abc","fromSeq":1042}   // 0 = 首次拉取
```

**响应信封** `SyncResponse`（domain/sync.rs:146，**注意外层信封字段无 rename，是 `status`/`message`/`data` 原样**）：
```rust
pub struct SyncResponse { pub status: String, pub message: Option<String>, pub data: Option<SyncResponseData> }
```
`status` 必须为 `"SUCCESS"` 才算成功（domain/sync.rs:233 / decode_sync_response:264）。

`SyncResponseData`（domain/sync.rs:140，camelCase）：
```rust
pub struct SyncResponseData { pub entries: Vec<SyncEntry>, pub server_time: i64 }   // wire: serverTime
```

`SyncEntry`（domain/sync.rs:119-135，camelCase）：
```rust
pub struct SyncEntry {
    pub channel_id: String,         // channelId
    pub kind: SyncEntryKind,
    #[serde(default)] pub events: Vec<ImEvent>,
    #[serde(default)] pub messages: HashMap<String, serde_json::Value>,  // msgId → Post JSON
    pub next_seq: Option<i64>,      // nextSeq（too_long 时 None）
    pub reset_to: Option<i64>,      // resetTo（仅 too_long 非 None）
}
```

`SyncEntryKind`（domain/sync.rs:73-80，**`#[serde(rename_all = "snake_case")]` — 与其它结构 camelCase 不同！**）：
```
NoChange→"no_change" | Events→"events" | Snapshot→"snapshot" | TooLong→"too_long"
```
语义（domain/sync.rs:66-72）：`no_change` gap==0；`events` 0<gap≤5000；`snapshot` 保留扩展（A2 未实现）；`too_long` gap>5000 客户端清表从 `reset_to` 重拉首屏。

`ImEvent`（domain/sync.rs:96-112，camelCase **但 created_at 单独 rename 为 `createAt`（少一个 d，mattermost 历史命名）**）：
```rust
pub struct ImEvent {
    pub event_seq: i64,        // eventSeq — per-channel 严格单调递增
    pub event_type: i8,        // eventType — 1=new 2=edit 3=delete 6=read
    pub msg_id: Option<String>,// msgId（read 事件可空）
    pub actor_id: String,      // actorId
    #[serde(rename = "createAt")] pub created_at: i64,   // ⚠️ createAt 非 createdAt
    pub payload: Option<serde_json::Value>,
}
```
**phantom 无独立字段**：通过 `messages.get(msg_id) == None` 识别（domain/sync.rs:90-92 + apply 分支）。

> 迁移坑（决策已沉淀 im_sync_wire_contract.md）：HTTP camelCase / WS snake_case 两套；`createAt` 不是 `createdAt`；`eventType` 数字枚举；外层信封 `status/data/message` 不 rename。`SyncEntryKind` 是唯一 snake_case 枚举值。

---

## 3. WS 接收 router 按 type 分发（`router.rs` + `message_loop.rs`）

### 3.1 接入点 `im_ws_client.rs`
- `ImWsHandler.on_message`（im_ws_client.rs:128）：解析 raw JSON → 提取 `action`（默认 ""）、`data`（默认 null）、`tracing`（顶层 `tracing` map，OTel 续链）、hello 时额外提 `data.connectionId`。组 `IncomingMessage{action,data,server_connection_id,tracing}` 经 mpsc `tx.send` 转给 message_loop。
- WS URL：`ws://{auth.mattermostWebSocket}/api/v4/websocket`（im_ws_client.rs:209-215）。
- 入站 header（build_headers im_ws_client.rs:259-272）：`apptype=cses, deviceId, device=WebBrowser, siteName=b, appVersion, cookieId, companyId?, envType?`。

### 3.2 message_loop（dispatcher + 4 worker）
- `WORKER_COUNT=4`（匹配 read_pool 4 连接），每 worker `WORKER_CHANNEL_CAPACITY=64`（message_loop.rs:22-25）。
- `route_to_worker`（message_loop.rs:103）：按 `extract_channel_id` 哈希（`DefaultHasher`）取模分片；无 channel_id（hello / post_map_config）固定 worker 0。**同 channel 串行**保证 cursor 状态机一致性。
- `classify`（router.rs:20）：`hello | post_segment | post_segment_lazy` → **Batch**（攒 200ms flush）；其余 → **Realtime**（立即处理）。worker_loop 200ms `flush_interval`（message_loop.rs:239）。
- Realtime → `handle_realtime`，Batch → 攒 `batch_buffer` 到 200ms tick → `flush_batch`。两者都：`router::route` → 对返回的每个 `(event_name, payload)` 调 `event_bus::emit_im(app, event_name, payload, msg.tracing)`。

### 3.3 `extract_channel_id`（router.rs:69）分片键提取规则（迁移端复用同语义）
1. `data.channelId` 非空（post/schedule/close 等）
2. channel 类（`increment_channel`/`increment_channel_end`/`update_channel`/`channel_created`/`channel_member_update`）→ `data.id`
3. segment 类（`post_segment`/`post_segment_lazy`）→ `data.segmentId` 按 `_` rsplit，取 `channelId_day_index` 的 channelId 段
4. hello / post_map_config / post_segment_finish → None（→ worker 0）

### 3.4 router.rs:28 `route` 完整 action → handler 分发表

| action | handler | 产出 emit 事件 |
|--------|---------|----------------|
| `hello` | hello::handle | `im:connection:established` + `im:channels:loaded`(增量响应) |
| `increment_channel` | channel::handle_increment | `im:channel:increment` + 异步 `im:post:increment*` |
| `increment_channel_end` | channel::handle_increment_end | `im:channel:update`(子topic) / `im:todo:updated` + 触发 proactive sync |
| `update_channel` | channel::handle_update | `im:channel:update` |
| `channel_created` | channel::handle_create | `im:channel:created` |
| `channel_close` | channel::handle_close | `im:channel:closed` |
| `channel_member_update` | channel::handle_member_update | `im:channel:member-updated` |
| `update_channel_member_nickName` | channel::handle_member_nickname_update | `im:channel:memberNickname`（直接 emit_im）|
| `post` | post::handle | `im:post:received` + result.channel_emits（如 `im:channel:update-by-post`）|
| `post_read` | post::handle_read | `im:post:read` |
| `post_update` | post::handle_update | `im:post:updated` |
| `posts_update` | post::handle_batch_update | `im:post:batch-updated` + channel_emits |
| `post_schedule_created` | schedule::handle_created | (schedule emits) |
| `post_schedule_canceled` | schedule::handle_canceled | (schedule emits) |
| 未知 | — | 仅 debug log `im.router.unknown_action` |

---

## 4. event_bus：单通道回吐事件给前端（`event_bus.rs`）

- 所有 `im:*` 业务事件统一发往**唯一内部 channel** `im:__bus__`（`BUS_CHANNEL` event_bus.rs:20）。前端 `tauri.service.ts` 的 bus dispatcher 按 payload 内 `channel` 名二次分发。
- 设计目的：N 个 IM channel 订阅 = N 次 IPC 注册压缩为 1 次（实测 16→1，省 ~190ms 启动）。
- 信封 `BusEvent`（event_bus.rs:30）：
  ```rust
  struct BusEvent<'a> {
      channel: &'a str,        // 原 im:* 事件名
      payload: &'a Value,
      #[serde(skip_serializing_if = "Option::is_none")] tracing: Option<&'a Value>,
  }
  ```
- `emit_im(app, channel, payload, tracing)`（event_bus.rs:44）：`app.emit("im:__bus__", &envelope)`。约束：`channel` 必须 `im:` 开头且**不**以 `im:__` 开头（grep 守护，未硬断言）。
- `tracing` 透传：来自 `IncomingMessage.tracing`（服务端 broadcast 帧顶层 `tracing` map），前端 `ImEventBusDispatcher.dispatch` 据此起 `ws.consume` CONSUMER span 续链。同一 WS 帧被 router 拆多事件时共享同 trace_id（fan-out）。

---

## 5. 增量同步：cursor / gate 状态机 / 心跳补偿（核心，迁移端必读）

### 5.1 本地 cursor 表 `channel_event_cursor`（tables/channel_event_cursor.rs）
DDL（channel_event_cursor.rs:28，前端 SQLite 自动 `CREATE TABLE IF NOT EXISTS`，无需手动）：
```sql
channel_event_cursor( channel_id TEXT PRIMARY KEY, last_event_seq INTEGER DEFAULT 0, updated_at INTEGER DEFAULT 0 )
```
- `upsert_cursor`（:49）：`ON CONFLICT DO UPDATE SET last_event_seq = MAX(existing, excluded)` —— **DB 层 MAX guard 单调递增、O(1)、不回退**。
- `get_cursor`（:85）缺省返回 0。
- struct `ChannelEventCursor`（:13）`#[serde(rename_all="camelCase")]` → `channelId/lastEventSeq/updatedAt`。

### 5.2 per-channel gate 状态机 `ChannelGateState`（state.rs:17）
内存态（`ImState.gate_states: tokio::Mutex<HashMap<String, ChannelGateState>>` state.rs:302）：
```rust
pub struct ChannelGateState {
    pub cursor: i64,                        // 已确认最新 event_seq（内存 cursor）
    pub buffer: BTreeMap<i64, ImEvent>,     // 乱序帧缓冲（seq 保序）
    pub gate_started_at: Option<Instant>,   // None=未激活
    pub gate_task: Option<JoinHandle<()>>,  // 1s timer 任务（可 abort）
}
```
关键方法（**WS 路径 vs sync 路径分工是迁移核心区分点**）：
- `flush_contiguous`（state.rs:59）：**WS-帧路径**严格 `cursor+1` 链 flush，gap=可能丢帧必须等齐。
- `flush_up_to(target)` / `peek_up_to` / `commit_up_to`（state.rs:98/132/153）：**sync-响应路径**跳合法稀疏空洞（CACHE>1 残留/删事件/分区空洞）直推 `next_seq`。`peek`+`commit` 分离实现「**写成功才推 cursor**」（2026-06-05 决策#2）。
- `peek_contiguous`（state.rs:141）：sync 缺 next_seq 的回退。

> **迁移铁律**：WS 帧 gap=严格 +1（怕丢帧）；sync 响应=权威全量区间，跳空洞推到 next_seq（否则卡第一个稀疏空洞→每次重连重拉→sync 风暴，2026-06-03 实测根因 state.rs:86-97）。

### 5.3 WS 推送入口 `on_ws_event`（post.rs:457）三路分支
1. `k == cursor+1`：顺序到达 → 推进 cursor、`flush_contiguous` flush buffer、`cancel_gate`、`write_cursor_to_db`。
2. `k <= cursor`：重复 drop（幂等，info 级日志）。
3. `k > cursor+1`：乱序 → `buffer.insert(k, ev)`，若 gate 未激活 spawn 1s timer → `on_gate_fire`。

惰性 seed（post.rs:469-495）：首次见该 channel 从 SQLite `load_persisted_cursor` 灌入内存 cursor，避免每会话首事件 cursor=0 触发全量重拉。收到任何 push 先 `heartbeat.touch_push(ch_id)`（心跳活跃戳，O(1)）。

### 5.4 gate fire → sync apply `on_gate_fire` / `on_gate_fire_inner`（post.rs:630 / 770）
- `on_gate_fire` 是带 `gate_inflight` 守卫的 BoxFuture wrapper（test-and-set 去重；finally remove；续拉 spawn 在 remove 之后避免被自身守卫挡掉）。BoxFuture 因自递归续拉无法推断 Send（post.rs:621-629）。
- inner 读 cursor → `sync_channels(cursors=[{ch_id, from_seq=cursor}])` → 按 `entry.kind`：
  - **NoChange**：cursor 推到 next_seq + 写盘。
  - **TooLong**：cursor 重置为 `reset_to-1`、清 buffer、`emit_im("im:sync:too_long", {channelId, resetTo})` 让 Angular 清表重拉首屏（post.rs:893-928）。
  - **Events/Snapshot**（post.rs:931）：events 插 buffer → `peek_up_to(next_seq)`（只读）→ `batch_write_events` 锁外批量落库 → **db_ok 才**升序 `apply_event_with_messages(skip_msg_write=true)` + `commit_up_to` 推进 cursor + 写盘；db_ok=false 则 cursor/buffer 原地不动、不 emit、下次 re-pull（最终一致性）。Events 页满（`batch_len >= 500`，服务端 `SyncEventsPerEntry=500` app/sync.go:79）返回 true 续拉下一页。

### 5.5 apply dispatch（post.rs:1146 `apply_event_with_messages`）→ emit 表
phantom 判定：`messages.get(msg_id) == None` → 仅推进 cursor 不渲染（info 级日志，post.rs:1270-1280）。

| event_type | 可见(msg in map) emit | phantom/缺失 | 持久化（WS 路径 skip_msg_write=false）|
|-----------|----------------------|--------------|----------|
| 1 new | `im:post:received` `{data:{channelId,eventSeq,msgId,actorId,post}}` + 离线未读 `update_channel_from_post`→channel_emits | 仅 cursor 推进 | `batch_upsert` |
| 2 edit | `im:post:updated` `{channelId,eventSeq,msgId,post}` | noop | `update_content_by_message_id`（保留 read_bits）|
| 3 delete | `im:post:deleted` `{channelId,eventSeq,msgId,actorId}` | — | `mark_revoked_by_message_id`(revoke=1) |
| 6 read | `im:channel:read_echo` `{channelId,eventSeq,actorId,msgId}` | — | read_bits 由 batch 路径 |
| other | warn，cursor 推进不动 UI | — | — |

> sync 路径 `skip_msg_write=true`：消息体已由 `batch_write_events` 分桶批量落库（type1 batch_upsert / type3 batch_revoke / type6 batch_apply_read / type2 逐条 update_content），apply 只负责 emit + CAP-9 离线未读（post.rs:1036-1042 / 1204-1249）。

### 5.6 在线 WS 帧如何喂 cursor（post.rs handle/handle_update/handle_batch_update）
- `handle`（post.rs:42）：渲染后 `build_ws_sync_event_from_post(post, data, "posted")` 抽 `event_seq` → `tokio::spawn(on_ws_event)`。
- event_seq 三级 fallback（post.rs:364-380，**迁移端注入点契约**）：① 顶层 `data.event_seq`（权威，post_core.go:424/559）→ ② per-post `props.channel_event_seq`（posted 冗余）→ ③ 顶层 `channel_event_seq`（legacy）。
- `handle_batch_update`（posts_update，post.rs:266-306）：event_seq 在 **TOP-LEVEL `data.event_seq`**（`msg.Add("event_seq", maxSeq)` post_core.go:559）；`revoke` 在 per-post（`data.posts[i].revoke`）→ `event_type = if revoke {3} else {2}`。

### 5.7 心跳 ping/pong gap 补偿（B4 — `all_hash.rs` + `state.rs::HeartbeatState` + `sync_compensation.rs` + `im_ws_client.rs`）

**ping（8s 间隔）** `build_ping_data`（im_ws_client.rs:96，**同步闭包，只读 parking_lot 镜像不碰 SQLite/async**）：
```
{ "action":"ping", "seq":<u64>, "data": { "cursors":[{"channelId","fromSeq"}], "allHash"?:<hex> } }
```
- `cursors` = 活跃群（open ∪ 近 5min push）cursor 列表，仅根群镜像里有的（`HeartbeatState.active_cursors` state.rs:272，`ACTIVE_PUSH_WINDOW=5min`）。
- `allHash` = 全量根群 cursor 滚动哈希（`root_cursors_snapshot` BTreeMap 升序 → `compute_all_hash`），空集省略（omitempty 对齐 Go）。

**`compute_all_hash`（all_hash.rs:25）FNV-1a 64bit — 两端 byte-exact 锚定向量（迁移单测必过）**：
- 输入按 channelId 升序，每项 `"<channelId>:<seq>"`，项间 `\n` join，整体喂 FNV-1a。
- `{"a":5,"b":10}` → `"87da68614226ef44"`（字节流 `"a:5\nb:10"`）；空/nil → `"cbf29ce484222325"`（offset basis）。
- 常量：`FNV_OFFSET_BASIS_64=0xcbf29ce484222325`、`FNV_PRIME_64=0x100000001b3`；输出 `{:016x}`。
- Go 对端：`server/channels/app/sync_reconcile.go::computeAllHash`。

**pong** 走 `ack_observer`（pong 是 `status==OK` ACK 被 runtime 吞，不进 handler，im_ws_client.rs:44-49）→ `handle_pong_compensation`（im_ws_client.rs:61）抽 `data.gaps[].channelId` + `data.hashMismatch` → spawn `compensate_from_pong`。

`compensate_from_pong`（sync_compensation.rs:31）：
- 活跃群 gaps 去重 + 节流（`MIN_COMPENSATE_INTERVAL=5s`，`heartbeat.should_compensate` test-and-set）→ 逐个 `compensate_channel`。
- `hashMismatch==true` → 全量根群（`heartbeat.root_channel_ids`）逐个补偿（共享同节流闸门避免双拉）。
- `compensate_channel`（post.rs:594）：seed cursor + 复用 `on_gate_fire`（含 inflight 去重 + 1s gate + 写成功才推 finally）。

**镜像写穿**（state.rs::HeartbeatState）：`set_root_cursor`（仅更新镜像已存在 key，避免话题群混入 allHash）由 `write_cursor_to_db` 写穿（post.rs:1523）；`seed_root_cursors` 在 increment 收尾后一次性灌（`seed_heartbeat_root_cursors` post.rs:1477）。

### 5.8 冷启动机制A：needSync 选择性 sync（hello + channel handler）
- hello（hello.rs:44）：清 `increment_fetched`/`need_sync_skip`、drain+abort 旧 `gate_states`（跨 logout/login 残留陈旧 cursor 修复 hello.rs:61-74）→ `build_root_cursors` 组根群 cursor → `get_increment_channels(timestamp, root_cursors)`。
- `handle_increment`（channel.rs:98）：`process_and_save` 落库 → 记 `increment_fetched`；帧顶层 `needSync==false` → 记 `need_sync_skip`（已追平跳过集，对齐 TS `pickNeedSyncChannelIds`「needSync !== false 才 sync」）。
- `handle_increment_end`（channel.rs:168）全局收尾：`run_post_backfill_owned` 补 getLatestPost；`to_sync = increment_fetched − need_sync_skip` → `proactive_resync_all_channels`（post.rs:727，逐 channel seed cursor + on_gate_fire 主动 sync）。

---

## 6. 本地 SQLite 表与落库（`tables/` + `repo/`）

`run_im_migrations`（tables/mod.rs:14）启动建表 + ALTER 补列（幂等）。`channel_bitmap` 表 **2026-05-27 PostSegment 整族下线已废弃**（mod.rs:67 启动 `DROP TABLE IF EXISTS channel_bitmap`）。

| 表 | 文件 | 主键 | 关键列（verbatim）| 用途 |
|----|------|------|------|------|
| `channel` | tables/channel.rs | id | 见下 channel_state | 会话列表 + 元数据；含离线同步列 `last_event_seq`（mod.rs:50 ALTER）|
| `message` | tables/message.rs | **temporary_id**（PK）| `id`(idx), `channel_id`(idx), `user_id`, `type`(rename), `message`, `props`, `viewers`, `read_bits`, `send_status`, `revoke:bool`, `reply_id/reply_root_id/reply_first_level_id`, `create_at`, `update_at` | 消息体；`#[serde(rename_all="camelCase", default)]` |
| `channel_event_cursor` | tables/channel_event_cursor.rs | channel_id | `last_event_seq`, `updated_at` | sync 断点续传 cursor（§5.1）|
| `channel_member` | tables/channel_member.rs | — | — | 成员归属（backfill_channel_members mod.rs:74）|
| `message_viewer` / `message_mention` | tables/ | — | — | 可见性 / @我 |
| `todo` | tables/todo.rs | — | — | 待办 |

> ⚠️ message 表 PK 是 `temporary_id` 非 `id`。用户自己发的消息本地 `id` 列可能为空串（仅 temporary_id），breaks 任何 `message.id`-keyed 逻辑（MEMORY self_message_empty_local_id；上拉锚点须 `messages.find(m=>m.id)`）。

落库路径：`repo/message_repo.rs`（`batch_upsert` / `update_content_by_message_id` / `mark_revoked_by_message_id` / `message_to_frontend_value`）、`repo/channel_repo.rs`、`repo/channel_member_repo.rs`。channel VO→DTO 纯计算在 `domain/channel_state.rs`（`channel_update_program` / `collect_present_fields`，含 DM 显示名解析、owner `{id,teamId}→{userId,teamId}` 变换、unread_count 的 `.or_else(local_dialog)` 守卫——依赖服务端降级时 omit unreadCount 而非回落 0）。

---

## 7. 迁移到 cses-im-server 的核对清单（客户端侧契约约束）

1. **sync/notify 响应**必须 byte-exact：外层 `{status:"SUCCESS", message?, data:{entries, serverTime}}`；entry camelCase `channelId/kind/events/messages/nextSeq/resetTo`；`kind` 是 snake_case 枚举；ImEvent camelCase 且 `createAt`（非 createdAt）+ `eventType` 数字。
2. **event_seq 注入点**：在线 post 帧顶层 `data.event_seq`（权威）；posted 帧 per-post `props.channel_event_seq` 冗余；posts_update 帧顶层 `event_seq=maxSeq`。客户端三级 fallback 依赖此三处任一存在。
3. **needSync** 是 channels/load/increment 响应帧顶层字段（与 channel 平级）；客户端「needSync !== false 才 sync」。
4. **心跳 ping `data`**：`{cursors:[{channelId,fromSeq}], allHash?}`；pong `data`：`{gaps:[{channelId}], hashMismatch:bool}`；status==OK ACK。
5. **allHash 算法**必须与 `all_hash.rs` byte-exact（FNV-1a 64，channelId 升序 `id:seq` join `\n`，16 位小写 hex），锚定向量 `a:5\nb:10 → 87da68614226ef44`。
6. **too_long** 语义：`reset_to` 是「重拉首屏起点」，客户端 cursor 落 `reset_to-1`。
7. **phantom**：服务端按可见性过滤 `messages` map，event 留在 events 但 msg_id 不入 map → 客户端推进 cursor 不渲染。
8. **SyncEventsPerEntry=500** 续拉契约：events.len()>=500 触发客户端续拉下一页。
9. 所有 path base `/api/cses`（mattermost csesapi 前缀）；im HTTP 客户端 connectionId header 由 hello 注入。

---

### 已知不一致 / 待补点（notes）
- `domain/sync.rs` 注释残留旧说法「SyncEntry/SyncResponseData 外层 snake_case（服务端 app/sync.go 显式 json:"channel_id"）」（domain/sync.rs:227 hint 字符串），但实际 struct 是 `rename_all="camelCase"` 且单测全验 camelCase（test_sync_entry_serializes_camel_case domain/sync.rs:485）—— **以 struct 定义 + 单测为准，hint 文案是历史遗留**。迁移端实现 sync/notify 响应序列化应输出 camelCase。
- `Snapshot` kind 在客户端 apply 与 Events 同分支处理但**不续拉**（A2 未实现快照下发，domain/sync.rs:78）。
- bitmap / segment 整族已下线（channel_bitmap 表 DROP、3 个 query 命令未注册、`services/segment_query` 业务路径删）；迁移端**无需**复刻 segment 协议。
