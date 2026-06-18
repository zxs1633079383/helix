# Helix sync-v2 设计（真 Go wire 契约 + 完成 spec）

> 合并自 `helix-sync-v2-real-go-wire-设计.md`（真 Go wire 契约 + 扭转流程 + 落地计划）+ `helix-sync-v2-完成-spec.md`（Phase C/D 收尾验收 spec），2026-06-17。
> 决策依据：2026-06-15 全链路 map（Go csesapi + cses-client tauri/前端 + helix 现状 6 路并行实测）。
> 拍板：**helix 适配现网 Go 真实 wire，不改 Go server**（ADR-008）。
> 行为真源：现网 `cses-client/src-tauri/src/features/im/`（GOAL §4.5），逐 `file:line` 已测绘。Go 服务端 `mattermost/server/channels/csesapi` 已服务生产 iOS/cses-client，**零改动**。
> 配套：REP 双向通道见 [`helix-eventbus-设计.md`](./helix-eventbus-设计.md)；core/im 契约见 [`helix-core-im-架构设计.md`](./helix-core-im-架构设计.md)；落地真相状态卡见 `crates/helix-im/docs/sync-v2-能力与缺口状态.md`（缺口卡，本文档引用其作现状）。

---

## 0. 当前总状态（2026-06-17）

> ✅ **sync-v2 数据路径已真 Go 端到端验证通过**：curl 发真消息 → Go post → WS `post` 帧（带 event_seq）→ helix parser 认 `post` action → gate apply → message 表落真实内容行 + temporary_id↔server_id 对账 + read_bits + cursor 推进 + `im:post:received` DomainEvent。三层 blocker 全清（grpc-deviceId / sync 风暴-并发闸 / WS-post-action-parser）。
>
> 这意味着 wire 设计 §1 与完成 spec §C1/C2/C3 的核心数据路径已不再是「待验」，而是「真 Go 跑通」。下文凡标「已验证通过」处即指此。剩余项见 §6 落地计划与 §7 验收清单。

---

## 1. 一句话定义

把现网 cses-client 命令式 IM 链路（hello 握手 → increment HTTP 触发 → WS 推 `increment_channel`×N + `increment_channel_end` → 对增量群 sync/notify 补漏 → 在线 gate 乱序 + 心跳补偿）**扭转**成 helix sans-IO 的 `step(Tick)->Vec<Effect>` 纯函数编排，I/O 全部由 driver 兑现。

**最大错位（必须纠正，已纠正）**：现网 increment 是「**HTTP 触发 → 后台 goroutine 算 → WS 单播推送**」非对称模型，HTTP 立即返回 `{status:SUCCESS}` 空壳。helix 早期误把它揉成一个抽象 `GET /sync/notify` 拉模型。本设计按真实 wire 拆回**两条独立链路**：increment（push）+ sync/notify（pull）。

---

## 2. 现网真实 wire 契约（SSOT，实现端逐字段对齐）

### 2.1 鉴权（所有 HTTP + WS 必带）

- **`cookieId` header = userId**（26 字符 mattermost id，如 `5ffd465e263864787b87aa01`）。`userId` header 优先、`cookieId` 兜底（Go `web/handlers.go:352-355`）。Go `RequireSession=false`，靠 cookieId→`UserData:<userId>` 解析 session。
- 其它：`companyId`（缺则 teamId 别名）、`connectionId`（hello 回注后注入）、`Cses-Track-Id`（链路追踪回写）。
- **归属裁决**：`cookieId=userId` 是**身份**，由 **driver 全局头注入**（host 登录后 `set_global_header`），core 永不认识 userId（GOAL §1.6）。`connectionId` 来自 hello 帧，**由 helix-im 解析后盖章进 Http header**。

### 2.2 WS 连接 + hello

- 地址：`ws://{host}:8065/api/v4/websocket?connection_id=&sequence_number=`（端口可配，首连不带 query）。
- **hello 帧**：`{action:"hello", data:{serverVersion, connectionId, pingInterval}}`，**服务端单向下发，客户端无需回 ack**，收到即就绪。
- 心跳：gorilla ping/pong，周期 = `hello.pingInterval`。客户端需实现 pong 保活（ping 心跳已实装，见 §6 Phase B-rest）。
- 帧统一信封：`{v:int(EnvelopeSchemaVersion=1), action:string, data:map, broadcast:{}, tracing:map, seq:int64}`。`v` 缺省按 0 legacy，未知 `v` forward-compat best-effort。

### 2.3 increment（push 模型）

- 触发：`POST /api/cses/channels/load/increment`，body `{timestamp:int64, ChannelId?:string(注意大写C), cursors?:[{channelId, fromSeq:int64}]}`（根群带 cursors，话题群不带）。
- **HTTP 立即返回** `{status:"SUCCESS", message:"loadIncrementChannel success"}`（**无数据**）。
- 真数据走 WS 单播（`NewWebSocketEventForUsers([userId])` 设 `broadcast.UserId`，hub 按 userId 投递）：
  - `{action:"increment_channel", data: IncrementChannel.ToMap()}`：含 `id/teamId/displayName/lastEventSeq/unreadCount/needSync/mentionList/urgentPostList`。逐个 channel 推。
  - `{action:"increment_channel_end", data:{channelId?}}`：本批结束信号。`data.channelId` 非空=某 parent 子topic批次结束；空=hello/重连整批结束。**空 channel/空增量/personal 用户也必发**（防前端永久 loading）。
- 兄弟端点 `POST /api/cses/channel/load/incrementByChannelId`：HTTP 直返单个 IncrementChannel，**不走 WS**。
- **`LastEventSeq`** = 该 channel 服务端当前 max `event_seq`，作离线同步 v2 cursor。

### 2.4 sync/notify（pull 模型，纯 HTTP）

- `POST /api/cses/channel/sync/notify`，body `{cursors:[{channelId, fromSeq:int64}]}`（非空且 ≤500）。**零 WS 推送**。
- 响应 `{status:"SUCCESS", data:{entries:[SyncEntry], serverTime:int64}}`。
- `SyncEntry = {channelId, kind, events:[ChannelEvent], nextSeq?:int64, resetTo?:int64, messages?:{msgId:Post}}`。
- `kind`（snake_case 字面量，不可改）：`no_change`（gap≤0，nextSeq=maxSeq）/ `events`（0<gap≤5000，≤500 条，nextSeq=末条）/ `too_long`（gap>5000，resetTo=maxSeq，清表重拉）/ `snapshot`（保留未启用）。阈值 `SyncEventsPerEntry=500`、`SyncGapTooLongThreshold=5000`。

### 2.5 ChannelEvent / event_seq（同步游标真源）

- `ChannelEvent = {id, createAt(★少一个d), channelId, eventSeq:int64, eventType:int8, msgId?, actorId, payload?}`。
- `eventType`：**1=new / 2=edit / 3=delete / 6=read**。
- `event_seq` 由每 channel 一个 PG sequence 分配，**append-only 表，禁 UPDATE/DELETE**。**可跳号**（业务行已 commit 但 event 追加失败容忍）→ client pts gate 必须容忍跳号，靠 sync 自补齐，**不能假设连续**。
- **event_seq 三级 fallback**（后端注入位置不统一）：① 顶层 `data.event_seq`（权威）→ ② per-post `props.channel_event_seq`（post 冗余）→ ③ 顶层 `channel_event_seq`（legacy）。
- 实时 `post` 帧 `data.event_seq` 与 increment 的 `LastEventSeq` **同源**（channel_event 表），统一 cursor。
  > ⚠️ 真帧 action 名是 **`post`**（非抽象的 `posted`/`posting`）——这是 WS-post-action-parser blocker 的根因，已修：parser 认 `post` action 帧。

### 2.6 phantom 语义

- `SyncEntry.messages` 是服务端按 caller viewers 过滤后的 map；**不在 map 的 msgId = phantom**（不可见但 **cursor 仍推进**）。sender 永远豁免。
- **铁律**：cursor 推进**独立于消息是否可见**，可见性是投影层（driver/UI）的事，不可见消息不可卡 cursor。

---

## 3. 理想扭转流程（sans-IO Tick/Effect 编排）

```
Phase 0 连接建立
  driver: NativeTransport::connect(ws_url, header{cookieId=userId})   ← 身份头 driver 全局注入
          └读任务→ Tick::Connected(TransportId(0))

Phase 1 hello 握手
  core(im) ← Tick::Connected   ⇒ conn=Connecting（等 hello，不 resync）
  driver   ← 服务端首帧 hello → Tick::Inbound(hello)
  core(im) ← Tick::Inbound(hello)  解析 connectionId ⇒
       Emit(im:connection:established{connectionId}) + ImState 存 connectionId + conn=Connected

Phase 2 increment HTTP 触发
  core(im) ⇒ Effect::Http{corr, POST /channels/load/increment,
              body:{timestamp, cursors:[{channelId,fromSeq}]}, header:[connectionId]}
  driver   → Tick::PortReply{corr, ok}   ← 仅 ack，无数据
  core(im) ⇒ Emit(im:channels:loaded)    （不在此等数据）

Phase 3 increment 推送（per-channel）
  driver  ← WS increment_channel × N → 每帧 Tick::Inbound
  core(im) ⇒ Persist{channel} + register cursor(LastEventSeq)
            + Emit(im:channel:increment{mentionList,urgentPostList})
            + 记 increment_fetched / needSync-skip

Phase 4 increment-End 收尾
  driver  ← WS increment_channel_end → Tick::Inbound
  core(im) ⇒ data.channelId 非空: Emit(im:channel:update)
            data 空(global end): Emit(im:todo:updated)
              + 对 (increment_fetched − needSync-skip) 每 channel
                ⇒ Effect::Http{POST /sync/notify}（进既有 gate/peek-commit 链）

Phase 5 吐指令到各 Driver
  engine_loop dispatch（统一 BoundedSpawner，见 §3.1）:
    Send  → NativeTransport::send   │ FfiTransport callback（M3）
    Http  → NativeHttp::request（必达/Block）  │ Ffi send_http callback
    HttpFire → NativeHttp::request（可丢/DropOldest）
    Emit  → NativeEventSink→app.emit('im:__bus__') │ Ffi on_domain_event→Dart Stream
    Persist→ host rusqlite（已通）  Timer→ TimerRegistry（已通）
```

### 3.1 背压根治：Effect Http/HttpFire 二分 + 统一 BoundedSpawner（2026-06-17 已落地）

sync fan-out（一次对 N 个 channel 发 `/sync/notify`）是 Http 高并发来源，早期 driver 的 ad-hoc semaphore + atomic + 泵顶 drain 三层各自为战，背压不传导回源 → sync 风暴。本会话根治：

- **`Effect` 模型新增 `HttpFire`（可丢·自愈）vs `Http`（必达·绝不丢）二分**（仿现有 `Persist`/`PersistFire`）：
  - `Http{corr}`：必达语义，绝不丢，队满则 **Block**（慢而不丢，背压传导回源 step）。**sync fan-out 的 `/sync/notify` 走 `Http`**——离线对账绝不能丢。
  - `HttpFire`：可丢语义，队满 **DropOldest**（自愈，只给显式声明可丢的请求，如纯遥测/可重算的轻量请求）。
- **driver 统一 `BoundedSpawner`**（取代旧 per-type ad-hoc 分发）：有界队列 + N worker + `on_overflow{Block, DropOldest}` + 独立 unbounded `reply_tx`。Persist / Http / HttpFire 共用同一套：
  - Persist：`N=1` / `Block`（WAL 单写者）
  - Http：`N=8` / `Block`
  - HttpFire：`N=8` / `DropOldest`
- **默认翻 Block**（背压传导回源，慢而不丢）；`DropOldest` 只给显式 `HttpFire`。删旧 semaphore + atomic + 泵顶 drain 三层。
- 跨模块公平性：登记 tripwire **不建**（YAGNI——当前单 IM 模块，跨模块公平性问题未出现）。

> 这是 sync-v2 fan-out 不再风暴的结构保证：cursor 推进只锚 Persist 成功的 PortReply（§3 铁律 2），fan-out 的 Http 必达 + Block 回压，sync 请求不会被悄悄丢掉导致漏补。

### 三条扭转铁律

1. **increment 的 Http 与 WS 推送是两条独立流**：core 不能等 Http reply 拿数据；PortReply 只是 ack。
2. **写成功才推 cursor**：cursor 推进只锚定 Persist 成功的 `PortReply`（触发 `MonotonicUpsert`），不依赖时钟/线程时序。sans-IO 化最易踩错点。
3. **WS 严格+1（flush_contiguous）vs sync 跳稀疏空洞（peek_up_to）两套 flush 不能混**：WS gap=可能丢帧（等齐）；sync gap=合法稀疏（CACHE 残留/删事件，跳洞推到服务端权威水位）。混用→cursor 卡第一个洞→sync 风暴（2026-06-03 实测根因）。

---

## 4. EventBus 24 事件枚举（分类 + 落地标注）

> helix 早期仅 3（`im:post:received` / `im:post:sending` / `im:sync:too_long`）。下表 = 目标全集；EventBus PUB/REP 信封契约见 [`helix-eventbus-设计.md`](./helix-eventbus-设计.md)。
> **REP 双向不是 EventSink 的事**：Http `corr`/`PortReply` 配对 + echo→reconcile + `read_echo` + ping/pong `gaps/hashMismatch` 补偿，见 §5。

| 分类 | 事件 | 备注 |
|---|---|---|
| 启动/生命周期 | `im:connection:established` | ✅ hello 握手后 emit（Phase A4）；UC-1 验收卡点 |
| | `im:channels:loaded` | hello 冷启动补齐 |
| 通知(消息) | `im:post:received` | ✅（已真 Go 验证：WS post 帧→落库→emit） |
| | `im:post:sending` | ✅ 乐观发送 |
| | `im:post:updated` | ✅ 编辑（type2，C3-core） |
| | `im:post:deleted` | ✅ 离线 sync type=3（C3-core） |
| | `im:post:read` | ✅ 在线/离线已读（type6，C3-core） |
| | `im:post:pin` | 功能性，暂缓 |
| | `im:post:batch-updated` | 在线批量编辑/撤回，暂缓 |
| | `im:post:increment`(+`-started/-empty/-failed`) | 首屏 latest 状态机（UI 徽标），暂缓 |
| 频道 | `im:channel:increment` | ✅ 含 mentionList/urgentPostList（Phase B） |
| | `im:channel:update` | ✅ increment_end |
| | `im:channel:update-by-post` | ✅ unread+N（C3-core） |
| | `im:channel:created` / `closed` / `member-updated` / `memberNickname` / `schedule-*` | 功能性，暂缓 |
| | `im:channel:read_echo` | 离线已读 type=6，actor==self 过滤（REP） |
| @我/与我相关 | `im:todo:updated` | about_me_post_ids→queryTodoList |
| 兜底 | `im:sync:too_long` | ✅ 前端清表重拉 |
| 数据格式 | 单总线 `im:__bus__` `{channel,payload,tracing?}` | ✅ 信封接线（`to_bus_envelope`，C3-core） |
| 🚫 运维消息 | 已下线（移出 roadmap，2026-06-16） | status_change/presence/quit_company 走 interceptor/UpdateService |

**信封硬约束**：Emit 跨 IPC/FFI 边界必须包成 `{channel:'im:xxx', payload, tracing?}` 单总线，前端按 channel 内存分发（对应 HX-C007 批处理）。array payload 不平铺（包 `{items:[]}` 防 Electron spread）。

---

## 5. REP 双向反馈落点（非 EventSink）

| 机制 | 落点 |
|---|---|
| sync 写成功才推 | `Effect::Http{corr}`（必达/Block） → `PortReply{corr,ok}` → 仅 ok 触发 commit cursor（peek_up_to/commit_up_to 两段式） |
| echo 对账 | echo 帧 `Tick::Inbound` → reconcile（tmp_id→server_id upsert 覆写 message.id+send_status + CancelTimer 15s + 清 pending_sends）。✅ 已激活（C1） |
| 离线已读回声 | `im:channel:read_echo`（type=6），driver/UI 层 actor==self 过滤 |
| 心跳 piggyback 补偿 | ping `data:{cursors,allHash}` → pong `data:{gaps,hashMismatch}` → 逐 gap 复用 sync 补漏（节流 5s）。需 core 提供「同步快照」能力（心跳闭包不能 await） |

> **不变量（R4-echo-gate）**：echo 对账与 channel gate 解耦——echo 对账只读 `pending_sends`/`corr_map`（按 temporary_id 对号），**从不读 `channels`**，故 `reconcile` 必须无条件先做，**不**门控 channel 是否已注册；落库 / 推进游标才门控注册。否则冷启动 send-before-cursor-seed 乱序下本端回声会被整段跳过 → PendingSend 残留 → 15s 后误标 UnSend。回归锚见 `tests/post_action_inbound_test.rs::echo_reconciles_even_when_channel_unregistered`。

---

## 6. 全量落地计划（A→B→C→D，落 dev-zlc）

> 用户拍板「全量真 Go wire 一次性」。实现仍按依赖排序分阶段提交（防超时丢工作 + 单 commit 单 type），但 scope = 全部。

### Phase A — 基础贯通（解锁真连 Go）✅ 完成（2026-06-16，commit e669f84..d7ef3c6）
- ✅ A1 `crates/helix-host-cli` 最小宿主 + `engine_loop` Send 接线 + `HashMap<TransportId, Arc<Mutex<NativeTransport>>>` 路由表。— `bf52b69`
- ✅ A2 入站 WS 帧 → `Tick::Inbound` 接线（读任务独立持 stream 半边）+ 本地 WS server 集成测试。connect 触发：host 主动 connect，重连状态机归 helix-im，Effect::Connect 暂不引入。— `b99fd7c`
- ✅ A3 driver 全局头 `cookieId=userId` + connectionId（im 盖章进 HttpRequest.headers，`connection_id_headers()` 单点）。— `d7ef3c6`
- ✅ A4 `parse_hello` 帧识别 + 握手时序（Connected→Connecting→收 hello→Connected）+ `emit im:connection:established`。修 🔴 旧 bug（Connected 即 resync→现网 Go 401）。— `d7ef3c6`

### Phase B — increment 真 wire ✅ 主体完成（2026-06-16）
- ✅ B1 parser：`increment_channel`/`increment_channel_end` 解析 + event_seq 三级 fallback。
- ✅ B2 `handle_increment`/`handle_increment_end` + `LastEventSeq` cursor 消费 + increment_fetched/needSync-skip 集 + global-end→proactive_resync。
- ✅ B3 `/sync/notify` GET→POST + `{cursors:[]}` body；SyncEntry 真字段（commit `6d83ad5`，HX-C008）：真 wire `{status,data:{entries:[SyncEntry]}}`、`eventSeq` 游标、续拉 `len≥500`、cursor 锚权威 `nextSeq`。
- ➕ **持久化层**：host-cli `IM_SCHEMA`（message/channel/channel_member）建表于 Application Support 多租户 tenant.db；cursor 真源 `_helix_monotonic`；发送路径落 message。
- ⏳ B2 increment_end **冷启动自愈**（`heal_incomplete_backfill_once`，post.rs:666-699 真源）——剩项。

### Phase C — 正确性 + EventBus ✅ 实现完成（commit `20334d2`，单测+对抗审绿；数据路径 2026-06-17 真 Go 验证通过）
- ✅ C1 echo→reconcile 激活（`handle_echo` 接进 Inbound，tmp_id→server_id 覆写 message.id+send_status）。**echo 真帧形态已核**（真 Go：WS `post` action 帧带 temporary_id → 匹配 → server post.id 覆写）。
- ✅ C2 phantom（`SyncEntry.messages` Post map→message 内容落库；msgId∉map→phantom 不落行 cursor 照推）+ type6 readBits 单调覆盖（不 +1，直接赋值）+ type2/3 edit/revoke。真源 cses-client `batch_write_events`（post.rs:1043-1134）。
- ✅ C3-core：sync/WS 驱动 mutation 事件（post:read/updated/deleted/channel:update-by-post）+ `im:__bus__` 信封接线（`to_bus_envelope`）。功能性事件(schedule/member 等)暂缓。

### Phase D — 真 Go smoke + golden fixtures ✅ 数据路径已通（2026-06-17）
- ✅ **真 Go 数据路径已端到端验证**：curl 发真消息 → Go post → WS `post` 帧 → parser 认 → gate apply → message 内容行 + tmp↔server 对账 + read_bits + cursor 推进 + `im:post:received` DomainEvent。三 blocker（grpc-deviceId / sync 风暴-并发闸 / WS-post-action-parser）全清。
- ⏳ golden fixtures 工具链：`tests/fixtures/real_go_frames/` 用 `HELIX_DUMP_FRAMES=1` 抓真 Go 帧补 echo / sync-with-messages / type2-3-6 序列 fixture（确定性 id/时间）——剩项。
- ⏳ UC-1 端到端脚本化（`scripts/dev-loop.sh` 扩）+ 全交付 6 crate 回归——剩项。

> **架构基座**：main↔dev-zlc 已合（`fcf3222`），采纳 **ADR-010 共享 HTTP/WS client**——`NativeTransport`/`NativeHttp` 改薄壳复用 `helix-driver-host` 的 `SharedWsClient`/`SharedHttpClient`。

### Phase B-rest（长连稳定性，与 sync-v2 数据正确性解耦）
- ✅ ping 心跳已实装（commit `488e094`）：self-arm 周期 timer + 单调 seq `ping_frame`，与 hello.pingInterval 对齐（修早期握手后 ~8s disconnect）。
- ⏳ 退避重连 / proxy·TLS·cert knobs / native·FFI registry 对称化（drift D3）。

### refactor（已落地，sync-v2 链路同款收敛）
- ✅ refactor A：入站二重分类 `FrameClass`+`EventKind` → 单 `InboundEvent` enum（parser `classify_inbound`），`Tick::Inbound` 分发单一收口。
- ✅ refactor B：5 张 `corr_to_*` HashMap → 单 `corr_map: HashMap<Correlation, CorrelationContext>`，`handle_port_reply` 一个 `match` 收口（Http/HttpFire/Persist/Request 共用）。

---

## 7. 验收清单（全绿才算 sync-v2 通过）

判据：**「连上 → 发出去对账回正 id → 离线 sync 补齐历史(含 phantom/已读/编辑/撤回) → 前端经 im:__bus__ 零改动消费」全链路真 Go 跑通** = sync-v2 验收。

- [x] C1 echo：`cargo test -p helix-im echo` + 真 Go 发消息回正 id（数据路径已验证）
- [x] C2 phantom/readBits/落库：单测 + 真 Go 离线 sync 有内容行 + read_bits 正确（数据路径已验证）
- [x] C3 事件+bus：emit 测试覆盖新事件 + DomainEvent 计数含 post:read/updated/deleted；`im:post:received` 真 Go 已出
- [ ] B2 自愈：冷启动剧本测试（剩项）
- [ ] D fixtures + UC-1：golden fixtures 工具链 + UC-1 脚本化 + 0 断连回归（剩项）
- [ ] 回归：交付 6 crate `cargo test` 全绿 + `scripts/gate.sh` + core 零运行时

**范围裁定**——暂缓（非 sync-v2 核心，另立）：C3 功能性事件（schedule/member/nickname/todo/channel:created-closed）· D1 FFI 句柄类型统一 · D3 native·FFI registry 对称化 · 退避重连。

---

## 8. 待决 / 风险 + expert-panel-review 结论落地

### 8.1 待决 / 风险
- **运维消息类别**：已下线（移出 roadmap，2026-06-16；重启需先立 ADR）。
- **connect 触发归属**：当前保持 host-cli 启动时主动 connect，`Effect::Connect{transport,url}` 意图暂不引入（A2 决策）。
- **event_seq 跳号容忍**：parser/gate 必须容忍非连续 event_seq，靠 sync 自补；不可假设 +1 连续（仅 WS 实时严格+1，sync 跳洞）。
- **跨会话 gate 残留**：现网每个 hello 窗口起点 drain gate_states + 清 dedup 集（2026-06-05 实测陈旧 cursor 卡首屏）。helix 需「会话边界 reset」入口。
- **心跳同步快照**：piggyback `build_ping_data` 在同步闭包跑（不能 await），需 core 提供同步可读镜像。

### 8.2 [critical·已修] mattermost ID 全族用 Id26，msgId 不能留 u64
- **发现**：§2.5 ChannelEvent.msgId、§2.6 phantom 的 `messages{msgId:Post}`、§5 echo 对账的 `server_id` 全是 26 字符 base32（含 a-z 非数字）。但 `ServerId(pub u64)` 仍是 u64 → reconcile 写 `Integer(server_id.0 as i64)` → 真 Go msgId 解析失败/截断 → **C1 echo→reconcile 整条死**，且与 ADR-009 自相矛盾。
- **落地**：B0b（commit `c1025ff`）`ServerId(u64)→ServerId(Id26)`，messages.id 列 `Integer→Text`。**ID 族铁律**：channelId / msgId / actorId / userId 等 mattermost 26 字符 ID 一律 Id26 + SQL TEXT；**唯独 `event_seq` 是 `Seq(u64)` 数值 cursor（Go `eventSeq:int64`），不是 ID，不迁移**。

### 8.3 [medium] one-shot timer 必须模块自清（core 无 one-shot 语义）
- **发现**：core 的 TimerMap 把**所有** timer 当 recurring（`Tick::Timer` 只 peek 不移除，唯一移除入口是模块发 `Effect::CancelTimer`）。one-shot timer 若**自然触发**而 handler 不补 CancelTimer → timer id 永久滞留 map（慢泄漏）。
- **落地铁律**（回链 core CLAUDE.md §4「one-shot timer 须 CancelTimer」）：

| timer 用途 | kind | arm 点 | 清除点 |
|---|---|---|---|
| 1s gate（缺口补拉） | one-shot | seq>cursor+1 buffer 时 | seq 补齐 cancel；**自然触发后 handler 自发 CancelTimer** |
| 15s 发送超时 | one-shot | send 命令吐 | echo 对账 cancel；**自然触发（标 UnSend）后 handler 自发 CancelTimer** |
| 5s 节流（pong 补偿） | one-shot | 补偿触发时 | **自然触发后 handler 自发 CancelTimer** |
| ping 心跳 | recurring | on_start / 握手后 | 与 hello.pingInterval 对齐；Disconnected 时 CancelTimer |

> 铁律：**one-shot timer 的 timeout handler 分支，在产出业务 Effect 的同时必须补 `Effect::CancelTimer(self)`**。否则 timer_map 慢泄漏。
