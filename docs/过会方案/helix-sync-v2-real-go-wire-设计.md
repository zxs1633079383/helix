# Helix sync-v2 真 Go wire 落地设计（实现级 SSOT）

> 决策依据：2026-06-15 全链路 map（Go csesapi + cses-client tauri/前端 + helix 现状 6 路并行实测）。
> 拍板：**helix 适配现网 Go 真实 wire，不改 Go server**（ADR-008），**先落本文档 → 后全量真 wire 一次性实现**。
> 行为真源：现网 `cses-client/src-tauri/src/features/im/`（GOAL §4.5）。Go 服务端 `mattermost/server/channels/csesapi` 已服务生产 iOS/cses-client，零改动。
> 配套：REP 双向通道见 [`helix-eventbus-rep-双向通道设计.md`](./helix-eventbus-rep-双向通道设计.md)；core/im 契约见 [`helix-core-im-架构设计.md`](./helix-core-im-架构设计.md)。

---

## 0. 一句话定义

把现网 cses-client 命令式 IM 链路（hello 握手 → increment HTTP 触发 → WS 推 `increment_channel`×N + `increment_channel_end` → 对增量群 sync/notify 补漏 → 在线 gate 乱序 + 心跳补偿）**扭转**成 helix sans-IO 的 `step(Tick)->Vec<Effect>` 纯函数编排，I/O 全部由 driver 兑现。

**最大错位（必须纠正）**：现网 increment 是「**HTTP 触发 → 后台 goroutine 算 → WS 单播推送**」非对称模型，HTTP 立即返回 `{status:SUCCESS}` 空壳。helix 当前误把它揉成一个抽象 `GET /sync/notify` 拉模型。本设计按真实 wire 拆回**两条独立链路**：increment（push）+ sync/notify（pull）。

---

## 1. 现网真实 wire 契约（SSOT，实现端逐字段对齐）

### 1.1 鉴权（所有 HTTP + WS 必带）

- **`cookieId` header = userId**（26 字符 mattermost id，如 `5ffd465e263864787b87aa01`）。`userId` header 优先、`cookieId` 兜底（Go `web/handlers.go:352-355`）。Go `RequireSession=false`，靠 cookieId→`UserData:<userId>` 解析 session。
- 其它：`companyId`（缺则 teamId 别名）、`connectionId`（hello 回注后注入）、`Cses-Track-Id`（链路追踪回写）。
- **归属裁决**：`cookieId=userId` 是**身份**，由 **driver 全局头注入**（host 登录后 `set_global_header`），core 永不认识 userId（GOAL §1.6）。`connectionId` 来自 hello 帧，**由 helix-im 解析后盖章进 Http header**。

### 1.2 WS 连接 + hello

- 地址：`ws://{host}:8065/api/v4/websocket?connection_id=&sequence_number=`（端口可配，首连不带 query）。
- **hello 帧**：`{action:"hello", data:{serverVersion, connectionId, pingInterval}}`，**服务端单向下发，客户端无需回 ack**，收到即就绪。
- 心跳：gorilla ping/pong，周期 = `hello.pingInterval`。客户端需实现 pong 保活。
- 帧统一信封：`{v:int(EnvelopeSchemaVersion=1), action:string, data:map, broadcast:{}, tracing:map, seq:int64}`。`v` 缺省按 0 legacy，未知 `v` forward-compat best-effort。

### 1.3 increment（push 模型）

- 触发：`POST /api/cses/channels/load/increment`，body `{timestamp:int64, ChannelId?:string(注意大写C), cursors?:[{channelId, fromSeq:int64}]}`（根群带 cursors，话题群不带）。
- **HTTP 立即返回** `{status:"SUCCESS", message:"loadIncrementChannel success"}`（**无数据**）。
- 真数据走 WS 单播（`NewWebSocketEventForUsers([userId])` 设 `broadcast.UserId`，hub 按 userId 投递）：
  - `{action:"increment_channel", data: IncrementChannel.ToMap()}`：含 `id/teamId/displayName/lastEventSeq/unreadCount/needSync/mentionList/urgentPostList`。逐个 channel 推。
  - `{action:"increment_channel_end", data:{channelId?}}`：本批结束信号。`data.channelId` 非空=某 parent 子topic批次结束；空=hello/重连整批结束。**空 channel/空增量/personal 用户也必发**（防前端永久 loading）。
- 兄弟端点 `POST /api/cses/channel/load/incrementByChannelId`：HTTP 直返单个 IncrementChannel，**不走 WS**。
- **`LastEventSeq`** = 该 channel 服务端当前 max `event_seq`，作离线同步 v2 cursor。

### 1.4 sync/notify（pull 模型，纯 HTTP）

- `POST /api/cses/channel/sync/notify`，body `{cursors:[{channelId, fromSeq:int64}]}`（非空且 ≤500）。**零 WS 推送**。
- 响应 `{status:"SUCCESS", data:{entries:[SyncEntry], serverTime:int64}}`。
- `SyncEntry = {channelId, kind, events:[ChannelEvent], nextSeq?:int64, resetTo?:int64, messages?:{msgId:Post}}`。
- `kind`（snake_case 字面量，不可改）：`no_change`（gap≤0，nextSeq=maxSeq）/ `events`（0<gap≤5000，≤500 条，nextSeq=末条）/ `too_long`（gap>5000，resetTo=maxSeq，清表重拉）/ `snapshot`（保留未启用）。阈值 `SyncEventsPerEntry=500`、`SyncGapTooLongThreshold=5000`。

### 1.5 ChannelEvent / event_seq（同步游标真源）

- `ChannelEvent = {id, createAt(★少一个d), channelId, eventSeq:int64, eventType:int8, msgId?, actorId, payload?}`。
- `eventType`：**1=new / 2=edit / 3=delete / 6=read**。
- `event_seq` 由每 channel 一个 PG sequence 分配，**append-only 表，禁 UPDATE/DELETE**。**可跳号**（业务行已 commit 但 event 追加失败容忍）→ client pts gate 必须容忍跳号，靠 sync 自补齐，**不能假设连续**。
- **event_seq 三级 fallback**（后端注入位置不统一）：① 顶层 `data.event_seq`（权威）→ ② per-post `props.channel_event_seq`（posted 冗余）→ ③ 顶层 `channel_event_seq`（legacy）。
- 实时 `posted` 帧 `data.event_seq` 与 increment 的 `LastEventSeq` **同源**（channel_event 表），统一 cursor。

### 1.6 phantom 语义

- `SyncEntry.messages` 是服务端按 caller viewers 过滤后的 map；**不在 map 的 msgId = phantom**（不可见但 **cursor 仍推进**）。sender 永远豁免。
- **铁律**：cursor 推进**独立于消息是否可见**，可见性是投影层（driver/UI）的事，不可见消息不可卡 cursor。

---

## 2. helix 现状 vs 目标错位表

> ✅ **本表「helix 现状」列为 2026-06-15 的旧抽象态，已被 B1/B2/B3 纠正（commit 6d83ad5，见 §6 Phase B）**——增量已拆 push/pull 两链路、WS 入站读 `event_seq` 三级 fallback、sync 端点 `POST {cursors}` 真 wire。下表保留作设计依据/对照。

| 维度 | helix 现状（抽象简化版，已纠正） | 目标（真 Go wire，已落地） |
|---|---|---|
| 增量模型 | 单一 `GET /sync/notify` 拉 | increment（push）+ sync/notify（pull）**两条** |
| WS 入站字段 | `data.seq` 一级 | `data.event_seq` + 三级 fallback |
| sync 端点 | `GET /sync/notify?channel_id&from_seq` | `POST /sync/notify` body `{cursors:[]}` |
| hello | on_start 只 Scan+ping，**不发/不收 hello** | Connected→等 hello 帧→connectionId→Connected |
| 鉴权头 | 全空（无 cookieId/connectionId） | driver 注 cookieId=userId；im 盖章 connectionId |
| increment 帧 | parser 不认（静默 drop） | increment_channel / _end 解析 + handler |
| echo 对账 | `#[allow(dead_code)]` 从不被调 | 接进 Tick::Inbound，tmp_id→正式id 覆写 |
| DomainEvent | 3 个 | 24 个 + `im:__bus__` 信封 |
| Effect::Send | engine_loop:474 tracing 占位，无 transport 路由表 | 路由表 + NativeTransport::send 接线 |
| host-cli | 不存在 | 最小宿主把 transport 接进泵 |
| FFI | 全 `todo!()` | M3 落地（本设计先 native 贯通） |

---

## 3. 理想扭转流程（sans-IO Tick/Effect 编排）

```
Phase 0 连接建立
  driver: NativeTransport::connect(ws_url, header{cookieId=userId})   ← 身份头 driver 全局注入
          └读任务→ Tick::Connected(TransportId(0))

Phase 1 hello 握手（新增，当前缺）
  core(im) ← Tick::Connected   ⇒ conn=Connecting（等 hello，不 resync）
  driver   ← 服务端首帧 hello → Tick::Inbound(hello)
  core(im) ← Tick::Inbound(hello)  解析 connectionId ⇒
       Emit(im:connection:established{connectionId}) + ImState 存 connectionId + conn=Connected

Phase 2 increment HTTP 触发（新 wire）
  core(im) ⇒ Effect::Http{corr, POST /channels/load/increment,
              body:{timestamp, cursors:[{channelId,fromSeq}]}, header:[connectionId]}
  driver   → Tick::PortReply{corr, ok}   ← 仅 ack，无数据
  core(im) ⇒ Emit(im:channels:loaded)    （不在此等数据）

Phase 3 increment 推送（新 wire，per-channel）
  driver  ← WS increment_channel × N → 每帧 Tick::Inbound
  core(im) ⇒ Persist{channel} + register cursor(LastEventSeq)
            + Emit(im:channel:increment{mentionList,urgentPostList})
            + 记 increment_fetched / needSync-skip

Phase 4 increment-End 收尾（新 wire）
  driver  ← WS increment_channel_end → Tick::Inbound
  core(im) ⇒ data.channelId 非空: Emit(im:channel:update)
            data 空(global end): Emit(im:todo:updated)
              + 对 (increment_fetched − needSync-skip) 每 channel
                ⇒ Effect::Http{POST /sync/notify}（进既有 gate/peek-commit 链）

Phase 5 吐指令到各 Driver
  engine_loop dispatch:
    Send  → NativeTransport::send（新接线）   │ FfiTransport callback（M3）
    Http  → NativeHttp::request               │ Ffi send_http callback
    Emit  → NativeEventSink→app.emit('im:__bus__') │ Ffi on_domain_event→Dart Stream
    Persist→ host rusqlite（已通）  Timer→ TimerRegistry（已通）
```

### 三条扭转铁律

1. **increment 的 Http 与 WS 推送是两条独立流**：core 不能等 Http reply 拿数据；PortReply 只是 ack。
2. **写成功才推 cursor**：cursor 推进只锚定 Persist 成功的 `PortReply`（触发 `MonotonicUpsert`），不依赖时钟/线程时序。sans-IO 化最易踩错点。
3. **WS 严格+1（flush_contiguous）vs sync 跳稀疏空洞（peek_up_to）两套 flush 不能混**：WS gap=可能丢帧（等齐）；sync gap=合法稀疏（CACHE 残留/删事件，跳洞推到服务端权威水位）。混用→cursor 卡第一个洞→sync 风暴（2026-06-03 实测根因）。

---

## 4. EventBus 24 事件枚举（分类 + 落地标注）

> helix 现仅 3（`im:post:received` / `im:post:sending` / `im:sync:too_long`）。下表 = 目标全集。
> **REP 双向不是 EventSink 的事**：Http `corr`/`PortReply` 配对 + echo→reconcile + `read_echo` + ping/pong `gaps/hashMismatch` 补偿，见 §5 / REP 专文。

| 分类 | 事件 | helix | 备注 |
|---|---|---|---|
| 启动/生命周期 | `im:connection:established` | ❌ | ⚠️ UC-1 验收卡点，全仓 0 命中 |
| | `im:channels:loaded` | ❌ | hello 冷启动补齐 |
| 通知(消息) | `im:post:received` | ✅ | |
| | `im:post:sending` | ✅ | 乐观发送 |
| | `im:post:updated` | ❌ | 编辑 |
| | `im:post:deleted` | ❌ | 离线 sync type=3 |
| | `im:post:read` | ❌ | 在线已读 |
| | `im:post:pin` | ❌ | |
| | `im:post:batch-updated` | ❌ | 在线批量编辑/撤回 |
| | `im:post:increment`(+`-started/-empty/-failed`) | ❌ | 首屏 latest 状态机（UI 徽标） |
| 频道 | `im:channel:increment` | ❌ | 含 mentionList/urgentPostList |
| | `im:channel:update` | ❌ | |
| | `im:channel:update-by-post` | ❌ | |
| | `im:channel:created` | ❌ | |
| | `im:channel:closed` | ❌ | |
| | `im:channel:member-updated` | ❌ | |
| | `im:channel:memberNickname` | ❌ | |
| | `im:channel:schedule-created/canceled` | ❌ | |
| | `im:channel:read_echo` | ❌ | 离线已读 type=6，actor==self 过滤（REP） |
| @我/与我相关 | `im:todo:updated` | ❌ | about_me_post_ids→queryTodoList |
| 兜底 | `im:sync:too_long` | ✅ | 前端清表重拉 |
| 数据格式 | 单总线 `im:__bus__` `{channel,payload,tracing?}` | ❌ | driver 只到 broadcast，未接信封 |
| 🆕 运维消息 | （现网 24 个里**无**，待拍是否新立一类） | — | status_change/presence/quit_company 是 WS action 但未进 im 总线 |

**信封硬约束**：Emit 跨 IPC/FFI 边界必须包成 `{channel:'im:xxx', payload, tracing?}` 单总线，前端按 channel 内存分发（对应 HX-C007 批处理）。array payload 不平铺（包 `{items:[]}` 防 Electron spread）。

---

## 5. REP 双向反馈落点（非 EventSink）

| 机制 | 落点 |
|---|---|
| sync 写成功才推 | `Effect::Http{corr}` → `PortReply{corr,ok}` → 仅 ok 触发 commit cursor（peek_up_to/commit_up_to 两段式） |
| echo 对账 | echo 帧 `Tick::Inbound` → reconcile（tmp_id→server_id upsert 覆写 + CancelTimer 15s + 清 pending_sends） |
| 离线已读回声 | `im:channel:read_echo`（type=6），driver/UI 层 actor==self 过滤 |
| 心跳 piggyback 补偿 | ping `data:{cursors,allHash}` → pong `data:{gaps,hashMismatch}` → 逐 gap 复用 sync 补漏（节流 5s）。需 core 提供「同步快照」能力（心跳闭包不能 await） |

---

## 6. 全量落地计划（A→B→C→D，落 dev-zlc）

> 用户拍板「全量真 Go wire 一次性」。实现仍按依赖排序分阶段提交（防超时丢工作 + 单 commit 单 type），但 scope = 全部。

### Phase A — 基础贯通（解锁真连 Go）✅ 完成（2026-06-16，commit e669f84..d7ef3c6）
- ✅ A1 `crates/helix-host-cli` 最小宿主 + `engine_loop` Send 接线 + `HashMap<TransportId, Arc<Mutex<NativeTransport>>>` 路由表（多连接形态）。— `bf52b69`
- ✅ A2 入站 WS 帧 → `Tick::Inbound` 接线（读任务独立持 stream 半边，不与 send 抢锁）+ 本地 WS server 集成测试。connect 触发：A2 决策保持 host 主动 connect，重连状态机归 helix-im（随后续 hello tick 驱动），Effect::Connect 暂不引入。— `b99fd7c`
- ✅ A3 driver 全局头 `cookieId=userId`（host-cli `set_global_header`）+ connectionId（im 盖章进 HttpRequest.headers，`connection_id_headers()` 单点）。— `d7ef3c6`
- ✅ A4 `parse_hello` 帧识别 + 握手时序（Connected→Connecting→收 hello→Connected）+ `emit im:connection:established`。修 🔴 旧 bug（Connected 即 resync→现网 Go 401）。— `d7ef3c6`
- **验收**：✅ host-cli 可连 `ws://...:8065/api/v4/websocket`（无 Go 时 connect 失败不 panic）；✅ 入站帧→Tick::Inbound 本地 WS 测试绿；✅ helix-im 51 测试绿（含 4 新 handshake replay）；✅ core 零运行时保持。**待真 Go 联调冒烟**（需起 Go 服务端 + 真 cookieId）。

### Phase B — increment 真 wire ✅ 主体完成（2026-06-16）
- ✅ B1 parser：`increment_channel`/`increment_channel_end` 解析 + event_seq 三级 fallback。
- ✅ B2 `handle_increment`/`handle_increment_end` + `LastEventSeq` cursor 消费 + increment_fetched/needSync-skip 集 + global-end→proactive_resync。（剩 B2 increment_end **冷启动自愈** 未做）
- ✅ B3 `/sync/notify` GET→POST + `{cursors:[]}` body；SyncEntry 真字段（commit `6d83ad5`，HX-C008）：真 wire `{status,data:{entries:[SyncEntry]}}`、`eventSeq` 游标、续拉 `len≥500`、cursor 锚权威 `nextSeq`。修了 139 个 `missing 'type'`。
- ➕ **持久化层**（678 对齐，本会话）：host-cli `IM_SCHEMA`（message/channel/channel_member）execv_raw 建表于 Application Support 多租户 tenant.db；cursor 真源 `_helix_monotonic`（`channel_event_cursor` 暂不建，M2 再引）；发送路径落 message。
- **验收**：单测全绿 + 真 Go :8065 直连冒烟通过（握手/increment/0 断连/入站达泵）。剩 UC-1 端到端（依赖 C 阶段）。

### Phase C — 正确性 + EventBus 全量 ❌ 未做（大头）
- ❌ C1 echo→reconcile 激活（`handle_echo` 仍 `#[allow(dead_code)]` 未接 Inbound）。
- ❌ C2 B5 phantom（`SyncEntry.messages` Post map→message 内容落库 + cursor 推进不渲染）+ B3 type=6 readBits。
- ❌ C3 emit 仅 6/24 事件 + `im:__bus__` 信封未接（driver 只到 broadcast）。
- **验收**：24 事件齐 + 前端零改动跑通（对齐现网 im:__bus__ 信封）。

### Phase D — 真 Go smoke + golden fixtures ❌ 未做
- ❌ D 录制真实帧 → Tick 剧本 → `tests/fixtures/`（确定性 id/时间注入）+ UC-2/UC-4。
- **验收**：现有 ROADMAP §三 M2 联调闸门全绿。

> **架构基座（本会话 merge）**：main↔dev-zlc 已合（`fcf3222`），采纳 **ADR-010 共享 HTTP/WS client**——`NativeTransport`/`NativeHttp` 改薄壳复用 `helix-driver-host` 的 `SharedWsClient`/`SharedHttpClient`，真 Go 硬修保住（Text 帧/握手头/不回 hello/入站 `Tick::Inbound`）。开放：D1 FFI 句柄类型统一、D3 native·FFI registry 对称化、ping/退避重连。

---

## 7. 待决 / 风险

- **运维消息类别**：现网 24 事件无专门运维事件，是否新立 `im:ops:*` 一类——待拍。
- **connect 触发归属**：当前 `Effect::Send{hello 帧}` 把「建连意图」塞进 frame payload，不干净。可选：新增明确 `Effect::Connect{transport, url}` 意图 vs 保持 host-cli 启动时主动 connect。Phase A2 拍。
- **event_seq 跳号容忍**：parser/gate 必须容忍非连续 event_seq，靠 sync 自补；不可假设 +1 连续（仅 WS 实时严格+1，sync 跳洞）。
- **跨会话 gate 残留**：现网每个 hello 窗口起点 drain gate_states + 清 dedup 集（2026-06-05 实测陈旧 cursor 卡首屏）。helix 需「会话边界 reset」入口。
- **心跳同步快照**：piggyback `build_ping_data` 在同步闭包跑（不能 await），需 core 提供同步可读镜像。

---

## 8. expert-panel-review 结论落地（2026-06-16，4 专家对抗证伪）

### 8.1 [critical·已修] mattermost ID 全族用 Id26，msgId 不能留 u64
- **发现**：§1.5 ChannelEvent.msgId、§1.6 phantom 的 `messages{msgId:Post}`、§5 echo 对账的 `server_id` 全是 26 字符 base32（含 a-z 非数字）。但 `ServerId(pub u64)` 仍是 u64，reconcile 写 `Integer(server_id.0 as i64)`——真 Go msgId 解析失败/截断 → **C1 echo→reconcile 整条死**，且与 ADR-009（channelId 已 Id26）自相矛盾。
- **落地**：B0b（commit `c1025ff`）`ServerId(u64)→ServerId(Id26)`，messages.id 列 `Integer→Text`。**ID 族铁律**：channelId / msgId / actorId / userId 等 mattermost 26 字符 ID 一律 Id26 + SQL TEXT；**唯独 `event_seq` 是 `Seq(u64)` 数值 cursor（Go `eventSeq:int64`），不是 ID，不迁移**。C 阶段 actorId（read_echo）按需补 `ActorId(Id26)`。

### 8.2 [medium] one-shot timer 必须模块自清（core 无 one-shot 语义）
- **发现**：core 的 TimerMap 把**所有** timer 当 recurring（`Tick::Timer` 只 peek 不移除，唯一移除入口是模块发 `Effect::CancelTimer`）。one-shot timer 若**自然触发**而 handler 不补 CancelTimer → timer id 永久滞留 map（`alloc_timer` 单调不复用 → 慢泄漏）。
- **落地（B-rest/C 实现铁律，回链 core CLAUDE.md §4「one-shot timer 须 CancelTimer」）**：

| timer 用途 | kind | arm 点 | 清除点 |
|---|---|---|---|
| 1s gate（缺口补拉） | one-shot | seq>cursor+1 buffer 时 | seq 补齐 cancel；**自然触发后 handler 自发 CancelTimer** |
| 15s 发送超时 | one-shot | send 命令吐 | echo 对账 cancel；**自然触发（标 UnSend）后 handler 自发 CancelTimer** |
| 5s 节流（pong 补偿） | one-shot | 补偿触发时 | **自然触发后 handler 自发 CancelTimer** |
| ping 心跳 | recurring | on_start / 握手后 | 与 hello.pingInterval（实测 60s）对齐；Disconnected 时 CancelTimer |

> 铁律：**one-shot timer 的 timeout handler 分支，在产出业务 Effect 的同时必须补 `Effect::CancelTimer(self)`**。否则 timer_map 慢泄漏。（更干净的做法是 core 给 Timer 加 one-shot 标志位让 engine 触发后自动 remove——属 core 改动需新 ADR，暂不做。）

> 另：心跳 ping 当前是占位帧 `{action:"ping"}` + 间隔与 hello.pingInterval（60s）未对齐——真 Go 冒烟见握手后 ~8s disconnect 疑似此因，B-rest 对齐 ping 帧格式 + 间隔。
