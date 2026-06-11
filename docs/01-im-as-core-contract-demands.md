# IM 作为 core layer 的"第一性需求方" —— core 契约需求清单 & 不绑定标尺

> **唯一事实来源**：2026-06-08 IM 源码实测真相（FRESH_IM_SOURCE）。本文所有结论可回指到该源码事实，**不使用任何训练记忆**（cses-client message-v3 旧知识已过时作废）。
>
> **本文的两个目的**
> 1. 列出 IM 模块要在 core layer 上跑起来，**对 core ports / module host / effect-event 总线的精确需求**（第一性需求方视角）。
> 2. 同时作为**验证标尺**：core 是否够用（IM 跑得通）且**不被 IM 污染**（core 不出现任何 IM wire 名 / schema / 编码）。
>
> 命名按项目约定：`helix-core`（核心层）/ `helix-im`（IM 模块）/ `helix-driver-native|wasm|ffi`（平台驱动）。

---

## 0. 三层定位回指（这是后面所有红线的依据）

```
平台驱动层 (core 之外)          | helix-core (通用, 不认识业务)      | 业务模块层 (helix-im 等)
-------------------------------|------------------------------------|------------------------------
tokio / sqlx / tungstenite     | ports: Transport/Storage/Clock/    | DomainEvent 多族 enum
reqwest / web-sys WebSocket    |        EventSink/Id/Random/HttpReq  | gate cursor 推进
SQLite / OPFS / IndexedDB      | module host: 注册/路由/生命周期     | flush_contiguous / flush_up_to
uniffi / wasm-bindgen          | effect/event 总线                  | peek/commit 对账 by temporaryId
                               | sans-IO 确定性执行壳                | seq<=cursor 去重 / C006 补发时序
```

源码印证三层确实存在：IM 当前把 I/O（`tokio::spawn`、`sqlx` upsert、`reqwest` HTTP sync、`tauri::AppHandle`）和纯逻辑（gate seq 判定、flush 选择、temporaryId 对账）**混在一起**（`post.rs` 同一文件既算 seq 又 `app.emit`）。Helix 的任务就是把这条线切开——纯逻辑入 `helix-im`，I/O 入 driver，中间只剩 `helix-core` 的 port trait。

---

## 1. IM 需要 core 提供哪些 ports

> 原则：port = **trait（接口）**，由平台驱动层实现，由 core 注入给模块。trait 方法用**通用语义命名**，**不出现 IM 词汇**。下表"IM 用在哪"列出源码触点；"core 该长啥样"列出去 IM 化后的通用签名。

### 1.1 `Transport`（出站帧 / 入站帧）

| IM 用在哪（源码） | core 该长啥样（通用，去 IM 化） |
|---|---|
| WS 入站单点 `message_loop.rs:73` dispatcher 喂帧；`im_send_message` 命令 (`commands.rs:37` `(action,data,seq)->bool`) 是 WS 发送原语 | `Transport` 只搬**不透明字节/帧**：`send(frame: OutboundFrame)`；入站由 driver 读出后**喂给 core 执行壳**（见 §3 入站）。core 不解析帧内容。 |

- core 视角：Transport 是"把 module 吐出的出站 effect 字节发出去"+"把网络读到的字节作为入站事件交回壳"。**core 不知道这是 WebSocket 还是别的**（native=tungstenite / wasm=web-sys WebSocket / ffi=平台 socket），也不知道帧 `type` 集合（`hello/post/post_read/...` 是 IM 私有）。

### 1.2 `Storage`（IM 需要的全部操作 → 抽象成通用 KV/批量/游标语义）

IM 实测用到的存储操作（源码）→ 必须能用 core 通用 Storage port 表达：

| IM 操作（源码） | 语义 | core 通用形态 |
|---|---|---|
| `message_repo.rs:48 batch_upsert` INSERT…ON CONFLICT(temporary_id) DO UPDATE SET id=excluded.id +23字段 | 热路径 O(1) 批量 upsert，冲突键值级覆盖 | `batch_upsert(table, rows, conflict_key)` —— **conflict_key 是参数**，core 不知道它叫 temporary_id |
| `update_content_by_message_id`（type2 edit） | 按业务键改字段 | `update(table, key, patch)` |
| `mark_revoked`（type3 delete） | 软删标记 | 同 `update`（patch 一个字段）—— core 不需要"revoke"概念 |
| `batch_apply_read`（type6 read） | 批量改已读 | `batch_update(table, keys, patch)` |
| `channel_event_cursor` upsert（PK channel_id, MAX guard, O(1)） | 单调游标持久化 | `cursor_upsert(scope_key, value)` with **MAX guard 语义**（见 §6）；core 提供"单调写"原语，不知道 scope 是 channel |
| `store:setItem` / `store:getItem`（乐观落库 / 读） | KV get/set | `get(table,key)` / `set(table,key,value)` |
| `peek_up_to` / `commit_up_to`（写成功才推 cursor） | 事务式"先写数据后推游标" | 见 §6：core 需要**"持久化成功回报"**契约，不是 Storage 单方法 |

- **关键**：Storage port 暴露的是 **batch_upsert / update / batch_update / get / set / cursor_upsert(单调)** 这几个**通用关系/KV 原语**。`temporary_id`、`message` 表名、23 个字段、`ON CONFLICT(temporary_id)`、`segment_id` 索引——**全是 helix-im 传进去的参数**，不进 trait 签名。

### 1.3 `Clock`（计时点）

IM 实测两个计时点（源码）：

| IM 计时点（源码） | core 通用形态 |
|---|---|
| 15s 发送超时 `message.service.ts:184 createSendTimeout` → 标 `unSend` | core 提供"定时器请求 effect"：module 吐 `ScheduleTimer{id, after_ms}`，driver 用 Clock 到点把 `TimerFired{id}` 喂回壳 |
| 1s gate timer `post.rs` seq>cursor+1 → buffer + 1s 后 `on_gate_fire` 触发 sync | 同上，另一个 `ScheduleTimer` + `CancelTimer{id}`（seq 补齐时 `cancel gate`） |

- core 视角：Clock 不是"模块直接读时钟"，而是 **module 吐 timer effect → driver 用真实 Clock 兑现 → 到点喂入 `TimerFired` 入站事件**。这样时间是**外部注入的确定性来源**（印证产品约束"时间由 host 注入"）。core 还需 `now_ms` 作为**入站参数**喂给每次 step（见 §3），让纯逻辑能给落库行打时间戳而自身不读时钟。

### 1.4 `EventSink`（吐 DomainEvent 给宿主 UI）

IM 实测吐出的事件（源码 `emit_im` event_bus.rs:44 → 单总线 `im:__bus__`，handler 返回 `Vec<(String,Value)>` String 含 `im:` 前缀）：
`im:post:received/updated/deleted/batch-updated/read`、`im:channel:read_echo/update-by-post`、`im:sync:too_long`。

| core 该长啥样 |
|---|
| `EventSink::emit(event: DomainEvent)`。**DomainEvent 是模块定义的强类型 enum**（见 §3 吐出），core 只负责把它送到宿主。`im:` 前缀、`im:__bus__` 通道名、wire 字符串——**core 完全不知道**。core 的 EventSink 搬的是"某个被序列化的 module 事件"，序列化为 `im:*` wire 名是 driver/adapter 边界的事，不是 core 的事。 |

### 1.5 `Id` / `Random`（确定性来源，host 注入）

IM 实测（源码）：`temporaryId` **主路径由 UI 组件生成**放进 `data.temporaryId`；仅 `sendDocumentMessage` 自己 `ObjectId.create()`。→ 直接印证 sans-IO「host/UI 注入 id」。

| core 该长啥样 |
|---|
| `Id::new_id()` / `Random::next()` 作为 **port**（确定性来源），但 IM 主路径里 id **由 UI 注入随用户意图带进来**（见 §3），core 纯逻辑**不主动生成 id**。port 存在是为"模块偶尔需要生成临时键"（如 doc 分支），且让 native/wasm/ffi 注入各自确定性实现以支持回放。**core 不知道生成出来的 id 叫 temporaryId、不知道它是 ObjectId 格式。** |

### 1.6 `HttpRequester`（出站 HTTP，sync notify 用）

IM 实测（源码）：发送主路径 `imHttp.post('/posts/create')`（HTTP 直发，不走 WS，不等回声）；gate fire → `reqwest` HTTP `POST /api/cses/channel/sync/notify(from_seq=cursor)`。

| core 该长啥样 |
|---|
| `HttpRequester::request(req: HttpRequest) -> effect`：module 吐"出站 HTTP request effect"，driver（native=reqwest / wasm=fetch / ffi=平台 http）执行，**响应作为入站端口响应喂回壳**。URL `/posts/create`、`/api/cses/channel/sync/notify`、`from_seq` 查询参数——**全是 helix-im 拼的 request body/url，core 只搬 HttpRequest 不透明结构**。 |

> 注：Transport（WS）与 HttpRequester（HTTP）是**两个 port**，因为 IM 实测发送双路径并存：主消息走 HTTP `/posts/create`，`im_send_message` 走 WS。core 不该把它们合并成"一个网络 port"，但也不该认识"哪个是发消息哪个是 sync"。

---

## 2. IM 模块的"纯确定性内核"边界

> 判据：**纯逻辑 = 给定相同入站序列 + 相同端口响应 + 相同 now_ms，必产生完全相同的出站序列（可回放、三端一致）**。一旦需要真实 I/O / 真实时钟 / 真实线程，就必须经 port 出去。

### 2.1 sans-IO 纯逻辑（留在 helix-im 内核，零 I/O / 零 await / 零 tokio）

全部来自源码、且全部是确定性状态机判定：

1. **gate cursor 推进判定**（`post.rs:457`）：
   - `seq == cursor+1` → apply + flush_contiguous + cancel gate + 推 cursor
   - `seq <= cursor` → **dup drop（幂等去重）**
   - `seq > cursor+1` → buffer + 请求 1s gate timer
   纯比较 + buffer map 操作，无 I/O。
2. **`flush_contiguous`（WS 语义）vs `flush_up_to`（sync 语义）选择与执行**：
   - contiguous：遇空洞停、严格 +1
   - up_to：跳稀疏空洞直达权威水位
   两者都是对内存 buffer 的纯遍历/筛选，决定"吐哪些 apply effect"。
3. **peek / commit 逻辑**（决策#2 2026-06-05）：peek_up_to 只算"该写哪些"（纯），commit_up_to 在**收到持久化成功端口响应后**才推 cursor（纯状态转移，见 §6）。
4. **对账 by temporaryId**：echo/sync 入站帧带 server id → 内核产出"以 temporaryId 为键的 upsert effect（id 值级覆盖）"+"撤 15s 超时 effect"。匹配纯逻辑（`messageDialog.service.ts:1501` 按 temporaryId 匹配 → 整条覆盖）。
5. **sync 响应分桶顺序铁律**（`batch_write_events`）：把 Events/Snapshot 拆成 type1 upsert / type2 edit / type3 revoke / type6 read **按固定顺序**产出 storage effect 列表。纯分类排序。
6. **续拉判定**：`events >= 500 → needs_continuation`（纯阈值比较 → 再吐一个出站 sync request effect）。
7. **TooLong 处理**：`cursor = reset_to - 1` + 吐 `too_long` DomainEvent（纯）。
8. **NoChange 处理**：`cursor → next_seq` + 清 gate（纯）。
9. **无自动补发**：失败只产出"标 unSend 的 storage effect"，**不产出任何重试 effect**（纯——没有定时重连补发状态机，2026-05-30 已删 Angular 第二套 sync 状态机，Rust 单 gate 独占 cursor）。

### 2.2 必须经 port 出去（不是纯逻辑，是 effect）

| 动作（源码） | 为什么不能在内核做 | 走哪个 port |
|---|---|---|
| 落库 batch_upsert / update / mark_revoked / batch_apply_read / cursor upsert | 真实 DB I/O，三端实现不同 | `Storage` |
| 发出站 WS 帧 / 接入站 WS 帧 | 真实 socket | `Transport` |
| HTTP sync notify `/sync/notify` 请求 + 响应 | 真实 HTTP | `HttpRequester` |
| 1s gate timer / 15s 超时计时 | 真实时钟 + 调度 | `Clock`（timer effect） |
| 吐 `im:post:*` 给 UI | 跨进程/跨 FFI/跨 wasm 边界 | `EventSink` |
| `tokio::spawn on_ws_event/on_gate_fire`（异步推 cursor） | 真实并发模型，**绝不能 bake 进 core**（wasm 单线程 / tokio multi-thread 在 wasm32 不可用） | driver 负责把"内核吐的 effect"在自己的 executor 上兑现；内核只吐 effect 不 spawn |

> **这条切割正是教训沉淀**：上一版 im_core 把 tokio/sqlx/tungstenite bake 进 core 是错的。在三栖目标下，2.2 里每一行都必须是 port，因为 native/wasm/ffi 实现完全不同。2.1 里每一行都不许碰 I/O，否则三端行为不一致、不可回放。

---

## 3. IM 喂入（入站）与吐出（effect/event）完整清单

> sans-IO 执行壳的形状：`step(inbound, port_responses, now_ms) -> Vec<Effect>`。下面是 IM 实测对应的完整 I/O 字典。

### 3.1 入站（喂给内核）

**A. WS 帧 type 全集（源码 router/dispatcher 列出的 14 类）**
`hello / increment_channel / increment_channel_end / update_channel / channel_created / channel_close / channel_member_update / update_channel_member_nickName / post / post_read / post_update / posts_update / post_schedule_created / post_schedule_canceled`
→ 内核解析为强类型入站 enum（**这层解析在 helix-im，不在 core**）。

**B. 用户意图（host → 内核）**
- `send`（chat-input sendMessage，带 UI 注入的 temporaryId）
- `read`（标已读）
- `resend`（15s 超时后用户**手点** unSend 重发——无自动补发）

**C. 端口响应（driver → 内核，effect 的回报）**
- sync 响应 4 kind：`NoChange / Events / Snapshot / TooLong`
- 落库结果：成功/失败（**peek/commit 决策的关键输入**，见 §6）
- HTTP 发送结果（`/posts/create` 成功/失败/超时）

**D. 时钟 tick**
- `TimerFired{id}`（15s 超时到 / 1s gate 到）
- 每次 step 携带 `now_ms`（给落库行打时间戳，host 注入）

### 3.2 吐出（内核 → driver，统一 `Effect` 流）

**A. DomainEvent（→ EventSink → UI）** —— 强类型 enum，对应源码 emit 集：
`PostReceived / PostUpdated / PostDeleted / PostBatchUpdated / PostRead / ChannelReadEcho / ChannelUpdateByPost / SyncTooLong`
（→ driver 序列化为 `im:post:received` 等 wire 名）

**B. 存储操作（→ Storage）** —— 分桶 type1/2/3/6 顺序铁律：
- type1 new：`batch_upsert`（可见→落库+unread；phantom→仅推 cursor 不渲染）
- type2 edit：`update_content`
- type3 delete：`mark_revoked`
- type6 read：`batch_apply_read`
- cursor：`cursor_upsert`（写成功才推，见 §6）

**C. 出站（→ Transport / HttpRequester）**：
- WS 出站帧（`im_send_message` 原语）
- HTTP sync request（`/sync/notify?from_seq=cursor`）
- HTTP 续拉 request（needs_continuation）

**D. 计时器请求（→ Clock）**：
- `ScheduleTimer{15s 超时}` / `CancelTimer`（echo 到达撤超时）
- `ScheduleTimer{1s gate}` / `CancelTimer`（seq 补齐 cancel gate）

> **core 只认识抽象 `Effect` 枚举的形状**（Emit / Persist / Send / HttpRequest / ScheduleTimer / CancelTimer），**不认识每个 variant 里 IM 填的内容**。

---

## 4. 绝不能渗进 core layer 的东西（通用性污染红线）

> 验证 core 不与 IM 绑定的硬标尺：以下任意一项出现在 `helix-core` 的源码/类型/trait 签名里 = **污染，必须打回**。

| 红线项（IM 私有） | 为什么是污染 | core 里正确形态 |
|---|---|---|
| `im:` wire 名（`im:post:received`、`im:__bus__`、`im:sync:too_long`） | 业务事件命名 | core 只搬 `DomainEvent`（模块定义的 enum）；wire 名在 adapter 序列化时才出现 |
| `temporaryId` 语义（作为对账 PK / id 值级覆盖） | IM 对账机制 | core 的 batch_upsert 只接 `conflict_key: &str` 参数，不知道叫什么 |
| Mattermost 帧格式 / 14 类 `type` 字符串 | IM wire 协议 | core Transport 搬不透明帧；type 解析在 helix-im |
| `channel_event_seq` / per-channel cursor 概念 | IM 的事件序号设计 | core 提供"单调 cursor_upsert(scope_key,value)"通用原语，不知道 scope=channel、value=event_seq |
| 四类 event_type 编码（1 new / 2 edit / 3 delete / 6 read） | IM 事件分类 | core 不知道有"四类"；分桶在 helix-im 内核 |
| `ON CONFLICT(temporary_id)` / `message` 表名 / 23 字段 schema / `segment_id` 索引 | IM 专属 DB schema | core Storage 接表名+行+冲突键作参数；schema 知识全在 helix-im |
| `/posts/create`、`/sync/notify`、`from_seq` | IM HTTP 契约 | core HttpRequester 搬不透明 HttpRequest |
| `sendStatus` 取值（local/sending/sent/unSend/...）、15s/1s 具体数值 | IM 业务常量 | core Clock 接 `after_ms: u64` 参数；15000/1000 由 helix-im 传 |
| phantom post / 可见性 / unread 概念 | IM 业务规则 | core 不认识 |

**一句话标尺**：把 `helix-im` crate 整个删掉，`helix-core` 必须仍能编译、且 grep 不到任何上表词汇。

---

## 5. C006 / 对账 / temporaryId 在 sans-IO 模型下怎么表达

源码事实回指：C006（补发时序）成立；对账 by temporaryId；echo 与 sync **同一对账点**（都走 `im:post:received` → 同一 handler）；无自动补发（15s → unSend 手点）；temporaryId 由 UI 注入；id 值级覆盖（`ON CONFLICT id=excluded.id`）。

sans-IO 表达：

1. **id/now host 注入**：`send` 用户意图入站时**带着 UI 生成的 temporaryId** 和 step 的 `now_ms`。内核**不生成 id、不读时钟**，因此发送序列完全可回放。
2. **乐观落库 = 一个 Persist effect**：内核吐 `Persist{ batch_upsert, conflict_key=<注入键>, row=<带 temporaryId 的乐观行, sendStatus=local> }` + `ScheduleTimer{15000ms, id=<该消息>}`。
3. **echo/sync 到达 = 同一对账纯逻辑**：入站帧（实时 echo 或 sync Events，源码确认两者同走 received）带 server id → 内核产出**同一组 effect**：
   - `Persist{ batch_upsert, conflict_key=<同一键>, row=<server id 值级覆盖, sendStatus=sent> }`（对账落地 = upsert 覆盖，不是新建第二行）
   - `CancelTimer{ id=<该消息> }`（撤 15s 超时，对应源码 `clearSendTimeout`）
   - `Emit{ PostReceived }`
   → **echo 与 sync 走完全相同的 effect 序列**，这就是"同一对账点"在 sans-IO 下的形态（一个纯函数，两个入站来源）。
4. **C006（补发时序）**：15s `TimerFired` 入站 → 内核纯逻辑产出 `Persist{ update sendStatus=unSend }`，**不产出任何重试 effect**。后续仅当 `resend` 用户意图入站才重走步骤 2。补发时序由"入站事件顺序 + 撤超时的幂等去重（seq<=cursor drop / temporaryId 已 sent 则忽略迟到 echo）"保证确定性——这正是 C006 在状态机里的体现：**echo 先到则 CancelTimer 让 15s 永不 fire；timer 先到则标 unSend，迟到 echo 仍按 temporaryId 覆盖回 sent**，两种到达次序都收敛到一致终态。

---

## 6. cursor 的 peek/commit "写成功才推" 在 sans-IO 下如何表达

源码事实回指（决策#2 2026-06-05）：`peek_up_to`（只读不推）→ `batch_write_events` 锁外分桶 → **写成功才 `commit_up_to` + `write_cursor`**；`cursor_upsert` 是 MAX guard 的 O(1) upsert；两种 flush（contiguous 严格+1 / up_to 跳空洞）。

sans-IO 表达（**两段式 step**，确定性靠"effect 与回报配对"保证）：

**Step N（收到 sync Events 入站）**：
- 内核 `peek_up_to`：纯逻辑算出"该写哪些 event（分桶 type1/2/3/6 顺序）"+"如果全写成功，cursor 该推到 `target_seq`"。
- 吐出 effect：`Persist{events 分桶批}`（**带一个 correlation id 标记这批对应 target_seq**），但**此刻不吐 cursor_upsert**。内核内部状态：`pending_commit[corr] = target_seq`，cursor **暂不推**。

**Step N+1（收到 Persist 端口响应）**：
- 端口响应 = `PersistResult{ corr, ok }` 入站。
- `ok == true` → 内核纯逻辑 `commit_up_to`：吐 `Persist{ cursor_upsert(scope_key, target_seq) }`（MAX guard 由 Storage 通用原语保证单调），并把内存 cursor 推到 target_seq。
- `ok == false` → 内核**不推 cursor**（pending_commit 丢弃/保留以待重拉），下次 sync 仍从旧 cursor 拉 → 不丢事件。

**确定性如何保证**：
- cursor 推进**只发生在"持久化成功端口响应"这个确定性入站事件上**，不依赖时钟/线程时序 → 同一 (events 入站 + PersistResult 入站) 序列必产生同一 cursor 终值。
- `flush_contiguous` vs `flush_up_to` 的区别落在 **peek 阶段算 target_seq 的纯规则**上（WS 严格+1 遇空洞停 / sync 跳空洞到权威水位），两者都走同一 peek→persist→commit 两段式。
- "写成功才推"在 sans-IO 下 = **cursor_upsert effect 必须晚于 events Persist effect 一个 round-trip，且以 PersistResult.ok 为门**。core 提供的契约就是：**effect 必须可携带 correlation id，且端口响应能带回同一 correlation id + 成功标志**。这是 core 执行壳对所有模块的通用要求，**core 不知道这是 cursor、不知道 MAX guard 是为单调 event_seq**。

> core 侧需求收口：执行壳必须支持 **(a) effect 带 correlation id；(b) 端口响应作为入站事件携带 correlation id + ok/err；(c) 模块可在内核保存 pending 状态跨 step**。满足这三点，IM 的 peek/commit 写成功才推就能纯逻辑表达——这是 §6 对 core 契约的硬性增量需求。

---

## 7. 回指总表：core 契约需求 ← IM 源码事实

| core 契约需求 | 回指源码事实 |
|---|---|
| Transport 搬不透明帧 | `message_loop.rs:73` dispatcher / `im_send_message`(commands.rs:37) |
| Storage: batch_upsert(conflict_key 参数化) | `message_repo.rs:48` ON CONFLICT(temporary_id) +23字段 |
| Storage: update/mark/batch_update | apply_event type2/3/6 (`post.rs:1113`) |
| Storage: 单调 cursor_upsert(MAX guard) | `channel_event_cursor.rs` MAX guard upsert O(1) |
| Clock: timer effect (after_ms 参数) | 15s `message.service.ts:184` / 1s gate `post.rs` |
| EventSink: emit(DomainEvent enum) | `event_bus.rs:44` emit_im → `im:__bus__` |
| Id/Random port 但内核不主动生成 | temporaryId 由 UI 注入 (`messageSend` 主路径) |
| HttpRequester 搬不透明 request | `/posts/create` 直发 / gate fire `/sync/notify` |
| 入站: WS 帧 14 type | router/dispatcher type 全集 |
| 入站: 端口响应(sync 4 kind/落库结果) | NoChange/Events/Snapshot/TooLong + 决策#2 |
| 纯逻辑: gate seq 判定 + 去重 | `post.rs:457` seq==/<=/>cursor+1 |
| 纯逻辑: 双 flush | flush_contiguous(WS) vs flush_up_to(sync) |
| 纯逻辑: 对账 by temporaryId 同一点 | echo+sync 同走 received / `messageDialog.service.ts:1501` |
| 纯逻辑: 无自动补发 | 2026-05-30 删第二套状态机 / 15s→unSend 手点 |
| effect 带 correlation id + 端口响应回报 | peek/commit 写成功才推 (决策#2 2026-06-05) |

---

## 8. 结论（给 core 设计的"够用 + 不污染"判据）

- **够用**：上面 6 个 port（Transport/Storage/Clock/EventSink/Id+Random/HttpRequester）+ 执行壳的 `step(inbound, port_responses, now_ms)->Vec<Effect>` + **correlation id 配对契约**，足以让 IM 全链路（发送→乐观落库→echo/sync 对账→cursor gate 推进→TooLong 兜底→15s 超时）跑成纯确定性内核。
- **不污染**：删掉 helix-im，helix-core grep 不到 §4 任一红线词。所有 IM 私有（wire 名/temporaryId/schema/event_type 编码/帧格式）都以**参数**形式穿过 port，不进 trait 签名。
- **三栖命根子**：Transport/Storage/Clock 三个抽象**必须保留**（这是上一版教训的反面），因为 native(tungstenite/sqlx/tokio-timer) / wasm(web-sys WS/OPFS/wasm 定时) / ffi(平台 socket/SQLite/平台 timer) 实现完全不同——单平台下可砍，三栖下是命根子。
