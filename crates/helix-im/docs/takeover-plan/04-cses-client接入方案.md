## 04 — cses-client 接入方案（Phase P7，本轮只设计，零代码改动）

> **强调**：本轮 cses-client **不动任何代码**，仅产出接入设计。前面 P0-P6 都是 helix 侧实现，本文档是「helix 替换现网 IM 核心」的落地剧本。
>
> **本轮 finalize 依据（2026-06-22）**：以下 IPC 面**不再是 rv-tauri 草案推断**，而是按 P0-P6 真实落地代码逐一核对的 ground truth——命令名 grep 自 `crates/helix-im/src/outbound/`（distributed_slice 注册）+ `module.rs`（Tick::Command dispatch）+ `query.rs`（投影命令）；emit 事件 grep 自 helix-im `Effect::Emit` 与 driver 横切层；WS 事件 grep 自 `ws/handlers/`。草案与 ground truth 不符处已在 §5.5「勘误表」逐条标注。
>
> **接管终态（2026-06-23 更新）**：① 命令面 **89** 出站/写（`im_send_message` + 88 outbound，createPosts/CP 已补齐）+ 2 query；② `projection-schema.md` **已冻结**（C2，21 投影工厂字段集 + 强制对账测试）；③ `im_reconnect` 控制臂落地、悬空 `im_read_channel` 已删（F1 gap-1）；④ **cses-client 接入 CI-1~CI-3 已完成**（git-dep + Tauri 薄壳 + 前端接线，灰度可逆 legacy 保留，commit `a4cbe201..e2c4ad1e`）；⑤ **CI-4 真机联调待 Go 后端环境**（89 命令 × 8 阶段矩阵已固化为可执行测试清单）。

---

### 1. 替换方式（二进制替换 + IPC 兼容层）

1. **二进制替换**：helix 编出 `helix-driver-native` 作为 lib（当前是暴露 `run_engine_loop` / ports 实现的引擎库，**不自带** `#[tauri::command]` 壳）；cses-client `src-tauri/Cargo.toml` 依赖 helix；现网 `src-tauri/src/features/im/` 整目录退役（保留为 `features/im_legacy/` 备份）。
2. **Tauri 命令壳在 cses-client 侧薄写**：helix-driver-native 提供引擎装配（`NativeStorage`/`NativeHttp::with_crosscut`/`NativeTransport`/`NativeEventSink`/`run_engine_loop`）+ `bus_guard::guard_bus_envelope` 出口守护；cses-client `src-tauri` 写一层 `#[tauri::command]` 薄壳把 invoke → `Tick::Command{name,payload}` 送进引擎，并把引擎 `EventReceiver` 收到的 `DomainEventBytes` 经 `to_bus_envelope` + `guard_bus_envelope` → `app.emit("im:__bus__", env)`。**命令名 + emit 信封 byte-exact 对齐表 A/表 B/表 C**。
3. **IPC 兼容层**：`framework/boot.rs` 的 `generate_handler!` 列表换成「helix 薄壳命令集」；**渲染层订阅 + 投影命令名不变**（表 A）；service 层出站/写命令改语义 invoke（表 B，是前端 service **重写**非零改，rv-tauri A2 诚实区分）。

---

### 2. 灰度开关（env IM_ENGINE + 前端 service legacy feature flag 双写分支，D4）

- **后端开关**：env / `tauri.conf`：`IM_ENGINE=helix|legacy`；`boot.rs` 启动时按开关注册 **helix 薄壳命令集** 或 **legacy 命令集**（**同名命令二选一注册**——同名 `#[tauri::command]` 不可并存，按 env 条件 build handler 列表）。
- **前端开关（回滚边界配套，D4）**：前端 service 层保留 `legacy` feature flag 双写分支——helix 分支走语义 invoke + 订阅投影；legacy 分支走原 HTTP 直发 + `store:*` 直写。两分支由同一 env（经 `tauri.conf` 暴露给前端 `IM_ENGINE`）切换。**理由**：表 B 的 service 已改语义 invoke，纯后端切 legacy 注册会让「前端发的语义命令名」在 legacy 命令集里 command not found → 必须前后端成对切（见 §4 回滚）。
- **灰度粒度**：内部 dogfood（白名单 userId）→ 10% → 全量（具体下发机制开放决策 §6 待拍板）。

---

### 3. IPC 面真源（按 P0-P6 真实注册收口）

#### 表 A — 渲染层零改（信封 + 投影 + 订阅，byte-exact 保留）

| IPC 面 | 决策 | 真源 |
|---|---|---|
| 单总线 `im:__bus__` 信封 `{channel,payload,tracing?}` | **原样保留**（已是最优 N→1）| driver `to_bus_envelope` 产此形；`bus_guard.rs::guard_bus_envelope` 守护出口 |
| 前端 listen 的 **14 个 `im:*` 业务事件**（见 §3.1）| 保留 channel 名 | CoreEventManager dispatch 不变 |
| 投影命令 `im_query_messages_by_channel`（分页 limit，复用现网 `store_get_messages` 语义）| 替 `imDataSource:*` 拦截垫片 | `query.rs::build_message_query`（已落地，A4）|

#### 表 B — service 层重写（出站/写收归 Rust，前端 service 改语义 invoke，**非零改**）

| IPC 面 | 决策 | 真源 |
|---|---|---|
| **89** 个出站/写命令（`im_send_message` + 88 outbound HTTP-fire，见 §3.2）| 全下沉 Rust 语义命令 + driver 横切层（鉴权/401/断网/trace）| `outbound/registry.rs`（88 distributed_slice）+ `module.rs` send arm |
| `store:* updateItem/setItem` 主动写（channel `{unread...}` / message）| **收归 Rust** 经字段级 `BatchUpdate WHERE PK` 推完整 DialogItem | 业务下沉铁律 + B5 禁 RMW（写权全在 helix-im `Effect::Persist`）|
| `im_delete_all_dialogs`（测试用清库）| 保留命令名，helix 清内存 channel gate + emit `im:channels:loaded`（物理 truncate 缺原语，见 §5.5③）| `query.rs::emit_dialogs_cleared`（已落地）|
| `im_reconnect`（= `IMWS:reconnect` 连接控制）| 保留命令名，helix-im 产 `im:net:reconnect_requested` 控制信号 → driver 观察后触发 reconnect | 连接生命周期归属 driver（`NativeTransport`）；F1 gap-1 已落地 `module.rs` Tick::Command 臂 + `to_effect::emit_reconnect_requested`（commit `3dae942`）|

#### 表 C — **不实现 / 勘误命令**（凭空补 = 死命令，YAGNI 红线）

| 草案 claim 命令 | 真相 | 处置 |
|---|---|---|
| `im_get_state` | 现网 boot.rs `generate_handler!` **无**此命令（rv-tauri B1「漏项」claim 与 ground truth 不符）| **不实现**（§5.5①）|
| `ensure_channel_loaded` / `im_sync_channels` / `im_switch_active_channel` | grep `crates/*/src/` 空——**不是真命令**。channel 加载/sync 由 `Tick::Connected` + WS inbound 帧（`hello`/`increment_channel`）驱动，**非显式 invoke**（§5.5⑤）| **不实现**（前端切会话只需 invoke 投影命令 `im_query_messages_by_channel` 拿本地消息；首屏/增量由 WS 帧自动推）|
| `im_load_older_messages` | 实际映射 outbound `im_post_context`（HTTP `posts/postContext`，上拉锚点）| 前端上拉历史 invoke **`im_post_context`**（args `post_id`/`before`）|
| `im_read_channel` | **悬空别名已删**（行为真源 cses-client 无 read_channel 概念；唯一已读 outbound = `im_post_read`，read_echo 仅 sync apply gate 路径产非命令）| **不实现**——F1 gap-1 已从 `acl/from_tick.rs::accepts_tick` allowlist 删除（commit `3dae942`）|

#### 3.1 emit 事件全集真源（14 业务 + 2 驱动横切 + 1 总线）

helix-im `Effect::Emit` 实际产出的 `im:*` 事件（grep `crates/helix-im/src/`）：

```
im:post:received      im:post:updated     im:post:deleted     im:post:read
im:post:sending       im:channel:update   im:channel:created  im:channel:closed
im:channel:increment  im:channel:read_echo im:channels:loaded im:sync:too_long
im:connection:established                  im:messages:query_result
```

driver P1.5 横切层 emit（grep `crates/helix-driver-host/src/`）：`im:http:unauthorized`（401）· `im:net:offline`（断网）。

控制信号（`im:net:*` 族，非渲染数据，不计入 21 投影集，projection-schema §1.1）：`im:net:reconnect_requested`（`im_reconnect` 命令产，data `{}`，driver 观察后重连 `NativeTransport`，F1 gap-1）。

总线本身：`im:__bus__`（Tauri event 名；业务 channel 禁撞 `im:__` 前缀，`bus_guard` 守护防自环）。

> **数组 payload 横切约束（M4）**：投影/列表事件（如 `im:messages:query_result`）的 data 若为 JSON 数组，driver `guard_bus_envelope` 统一包成 `{"items":[...]}`——前端 dispatcher 按对象解构，不为「裸数组 vs 对象」两套分支。前端订阅这些事件时**按 `{items:[]}` 解**。

#### 3.2 出站命令全集真源（87 outbound + 1 send，分域）

`im_send_message`（special：乐观落库 + HTTP `posts/create` + 15s timer + WS echo 锚 `action=="post"`）+ 以下 **88** 个纯 HTTP-fire outbound 命令（`outbound/registry.rs` distributed_slice，**前端 invoke 参数 = UI 字段，禁含 wire-only key**，C1）：

- **posts 写族（P2 + CP，8）**：`im_urgent_post` `im_urgent_confirm` `im_urgent_cancel` `im_quick_reply` `im_update_post_props` `im_template_received` `im_revoke` `im_create_posts`（批量转发 `posts/createPosts`，camelCase `{posts,channelIds}`，CP 补齐）
- **posts 读族（P3a，17）**：`im_get_schedule` `im_post_context` `im_top20` `im_get_posts` `im_get_posts_after_index` `im_get_replies` `im_get_reply_branch` `im_query_todo_list` `im_get_updated_posts` `im_get_latest_post` `im_bookmark_create` `im_bookmark_delete` `im_bookmark_load` `im_announcement_accept_list` `im_announcement_list` `im_announcement_detail` `im_post_read_list`
- **channel 写族（P3b，含 schedule，17）**：`im_create_channel` `im_create_specify_owner` `im_make_topic` `im_channel_close` `im_channel_leave` `im_channel_member_change` `im_channel_member_role` `im_channel_member_notify` `im_update_member_nickname` `im_channel_change_info` `im_channel_change_source` `im_channel_change_picture` `im_channel_change_notice` `im_channel_change_top` `im_post_read` `im_create_schedule` `im_cancel_schedule`
- **user-misc 域（P5a，11）**：`im_list_users` `im_get_user_statuses` `im_team_upsert` `im_team_member_add` `im_team_quit` `im_get_all_modules` `im_load_send_notifications` `im_search_post` `im_search_user` `im_search_channel` `im_search_do`
- **bot-agent 域（P5b，35）**：`im_create_bot` `im_bot_callback` `im_bot_create` `im_bot_list_visible` `im_bot_scope_set` `im_bot_token_generate` `im_bot_token_list` `im_bot_token_revoke` `im_bot_visible_user_add` `im_bot_visible_user_list` `im_bot_visible_user_remove` `im_bot_agent_info` `im_bot_agent_get_user` `im_bot_agent_regenerate_token` `im_bot_agent_config_create` `im_bot_agent_config_get` `im_bot_agent_config_delete` `im_bot_agent_configs_enabled` `im_bot_agent_channel_add` `im_bot_agent_channel_remove` `im_bot_agent_channel_bots` `im_bot_agent_channel_available_bots` `im_bot_agent_team_channel` `im_bot_agent_team_members` `im_bot_agent_send_channel` `im_bot_agent_send_direct` `im_bot_agent_test_webhook` `im_agent_create` `im_agent_callback` `im_agent_send` `im_agent_start_session` `im_agent_join_channel` `im_webhook_config_get` `im_webhook_config_set` `im_webhook_test`

> 计数核对：8+17+17+11+35 = **88** outbound + `im_send_message` = **89** 出站/写命令（表 B）。`is_outbound` 注册表 = 88，`im_send_message` 单独走 send arm，`im_query_messages_by_channel`/`im_delete_all_dialogs` 走 query arm（不计入 outbound）。
>
> **createPosts 已收口（CP / F2 / gap-2，2026-06-23）**：`im_create_posts` 已注册（registry 88），端点 `posts/createPosts`（双段复数），body **camelCase** `{posts,channelIds}`。casing 经三方证据链定论（前端真实 caller = 上线 wire + Go handler 无 json tag 解构体 + 接口全景三处佐证；**无 live-capture curl** 留存——用户记忆中的「含 createPosts 的 curl」实为测试用例清单非抓包）。详见 [`真机curl真源.md` 附录 A](../csesapi-migration/真机curl真源.md)。helix `posts_relay.rs::CreatePostsCommand` 与三方证据零偏差，`cp_create_posts_test.rs` 7 用例锚此 casing。

#### 3.3 WS 入站事件全集真源（19 live + 3 dead-action no-op + 4 ignored）

helix-im `ws/handlers/` distributed_slice 注册（前端**不直接订阅 WS**——driver `NativeTransport` 收帧 → `Tick::Inbound` → helix-im parse + dispatch → 转成 §3.1 的 `im:*` emit）：

- **19 live**：`post` `posts_update` `post_update` `post_read` `post_pin` `post_schedule_created` `post_schedule_canceled` `increment_channel` `increment_channel_end` `update_channel` `update_channel_notice` `update_channel_member_nickName`（**camelCase quirk**）`channel_created` `channel_close` `channel_member_update` `channel_member_role_updated` `change_channel_approval`（**wire 无 Status 后缀**）`quit_company` `hello`
- **3 dead-action no-op**（P4 清理：注册但**不实现业务逻辑**，echo 真路径 = `post`）：`posted` `post_urgent` `post_edited`
- **4 ignored no-op**（前端态/呈现态，core 不消费）：`status_change` `multiple_channels_viewed` `typing` `presence`

---

### 4. 回滚策略

- **一键回滚**：`IM_ENGINE=legacy` 切回 legacy 注册 + 前端 service legacy 分支（feature flag 双写，§2）。**回滚边界 = 前端 service 与 boot.rs 注册成对**——非纯后端开关（表 B service 已改语义 invoke，回滚需前后端同步切 legacy 分支，否则语义命令名在 legacy 命令集 command not found）。
- **数据兼容**：helix 与 legacy 同写 SQLite `message`(PK=temporaryId)/`channel`/`channel_member`/`channel_event_cursor` 表，schema 对齐（helix 已移除 segment_id 死列，ADR-013；与 legacy `IF NOT EXISTS` 共存不冲突）→ 回滚不丢数据。
- **写权冲突防线**：灰度期**禁止 helix 与 legacy 同时对同一 PK 写**（双写 PK 冲突）——env 二选一注册保证同一时刻只有一套引擎写。

---

### 5. 联调验收 checklist

- **双端日志**：`/im-dual-endpoint-log-debug`（mattermost + cses-client）；4 段链路（emit → 前端接力 → HTTP 出站 → 服务端处理）。
- **e2e 场景**：
  - 257 channel 真 Go connect/increment/sync（验 SyncScheduler 全局窗口 ≤K 限流不风暴，B4）；
  - send 6/8 echo 对账（验 `action=="post"` 锚 reconcile，B3；非 `posted`）；
  - too_long → 清内存重拉首屏（**不清表**）→ Rust 拉 `im_get_latest_post` 投喂 `im:post:received`；
  - 切会话：invoke `im_query_messages_by_channel` 拿本地消息（**无 `im_switch_active_channel` 命令**，§5.5⑤）；
  - 上拉历史：invoke `im_post_context`（**非 `im_load_older_messages`**，§5.5⑤）；
  - 401/断网横切端到端（验 P1.5：emit `im:http:unauthorized` / `im:net:offline`）。
- **纯渲染回归 gate**：前端零业务逻辑——unread/readBits/lastPost/加急/回复全由 Rust 投喂；投影事件字段集 == cses-client `DialogItem` interface（验 C2 投影 schema 字段齐，见 §6 前置）。
- **emit 信封守护**：抓 `im:__bus__` 出口，断言每条 channel `im:` 开头且非 `im:__`（防自环）；数组 payload 须 `{items:[]}` 包装。

---

### 5.5 P6 落地后 IPC 面 finalize 勘误表（按 P0-P6 真实代码，2026-06-22）

> rv-tauri 草案的若干 claim 与 ground truth 不符，此处逐条勘误（已落地代码为准）。

**① `im_get_state` 不存在**：核对真实注册面（`module.rs` dispatch + `query.rs::is_query` + outbound registry）——**无** `im_get_state`。rv-tauri 01 文档表 B「`im_get_state` 补入兼容表（boot.rs 已注册）」与现网不符。helix **不实现**（凭空补 = 前端永不调用的死命令，违 YAGNI）。`im_delete_all_dialogs` 确实需保留（前端「清库」按钮调）。

**② 投影命令 `im_query_messages_by_channel` 经 `ScanSpec` additive 扩展落地**：现网是 `SELECT * FROM message WHERE channel_id=? ORDER BY create_at DESC, temporary_id DESC LIMIT ?`。P6 给 `ScanSpec` 加 **additive 可选字段** `filter: Option<(&'static str, SqlValue)>` + `order_desc_col: Option<&'static str>`（None = 旧全表语义零行为改变，HX-C004）。helix 走 `Scan(message, filter=channel_id, order_desc=create_at, limit clamp≤500)` → emit `im:messages:query_result`（data `{channel_id, messages:[...]}`）。**保真偏差**：现网二级排序 `, temporary_id DESC`（同毫秒簇确定化）未携带（`order_desc_col` 当前单列）——dump 投影用途下同毫秒次序非语义关键；若未来需要，`order_desc_col` 升 `Vec<&'static str>` 再 additive 扩。scroll 游标分页（before/after）走另一路 `im_post_context`（HTTP postContext），不在本投影命令。

**③ `im_delete_all_dialogs` 物理 truncate = 核原语缺口（接入前可选项）**：现网 `DELETE FROM channel` + `DELETE FROM channel_member`（无条件全表删）。core `StorageOp` 无 truncate 原语（`BatchDelete` 需 `scope_col=? AND key_col IN (...)`，表达不了删全表）。P6 实现 = **清 core 内存 channel gate + emit `im:channels:loaded` 刷新**（命令名被识别，前端不报 command not found）；**物理 `DELETE FROM` 留待补 `StorageOp::Truncate`/`DeleteAll` 原语**——不为单个测试命令现造原语（YAGNI/HX-C001）。M2 共享既有 tenant.db 时 legacy 仍可物理删；helix 接管后该测试命令降级为内存清+刷新（功能影响仅限「测试用清库」按钮的物理落库部分）。

**④ `createPosts` 已收口（CP / F2 / gap-2，2026-06-23）**：经三方证据链定论端点 `posts/createPosts`（双段复数）+ body **camelCase** `{posts,channelIds}`——**无 live-capture curl** 留存（用户记忆中「含 createPosts 的 curl」实为测试用例清单非抓包）；`im_create_posts` 已注册（registry 88），与现网零偏差。原「待真抓包 blocker」状态解除。详见 [`真机curl真源.md` 附录 A](../csesapi-migration/真机curl真源.md)。

**⑤ channel 加载/sync 无显式 invoke 命令（草案 prose 与实现差异）**：01/02 文档 prose 把 `ensure_channel_loaded`/`im_sync_channels`/`im_switch_active_channel`/`im_load_older_messages` 写成语义 invoke 命令。**实现真相**：grep `crates/*/src/` 这四个名字全空——channel 注册（`register_channel`）与 sync 调度全由 **`Tick::Connected` + WS inbound 帧**（`hello` 触发批量、`increment_channel` 触发 needSync 入 SyncScheduler 队）驱动，**前端无需为「加载/同步/切会话」发任何 invoke**。前端切会话只需 invoke 投影命令 `im_query_messages_by_channel` 读本地；上拉历史 invoke `im_post_context`（= 草案误名的 `im_load_older_messages`）。**本 finalize 以实现为准**：表 C 列入「不实现」，验收 checklist（§5）已据此改写。

---

### 6. 接入前置 checklist（设计期确认项）

- [x] **`createPosts` casing 已拍板**（CP / F2，camelCase `{posts,channelIds}`，三方证据定论无 live curl）；createUser/presence/灰度机制/覆盖率工具仍为开放决策（§6 待拍）
- [x] **`projection-schema.md` 冻结**（C2，commit `96ab4c3`）：21 个 `im:__bus__` 投影 emit 工厂的 `payload.data` 字段集冻结成 cutover 契约真源 + 26 用例强制对账测试（缺失/多余即 fail，HX-C011 可证伪实证）；§1.1 控制信号（`im:net:reconnect_requested`）与 21 投影集分开冻结。见 [`projection-schema.md`](../projection-schema.md)。
- [x] 真 Go fixture 抓全（所有 casing 陷阱命令，C5）—— P3a/P3b 已用真 Go fixture 比对；`createPosts` 经三方证据链定论（无 live curl，用前端真实 caller = 上线 wire 实证）
- [x] P1-P7 全 gate 绿 + 全仓 `cargo test -p helix-im` + host-cli e2e 通过（**已验**：F1 收尾 416 passed 0 failed）
- [x] 灰度 feature flag 前端双写分支就绪（回滚边界，§2/§4）—— cses-client CI-1~CI-3 已接入（commit `a4cbe201..e2c4ad1e`，IM_ENGINE 开关 + legacy 全程保留）
- [x] cses-client `src-tauri` 薄壳层落地（§1.2：invoke→Tick::Command + EventReceiver→guard→app.emit）—— cses-client CI-2 已落地（91 语义命令薄壳 + `im:__bus__` 总线桥，commit `774390e5`）；helix 侧 `guard_bus_envelope`/`run_engine_loop`/ports 备齐
- [ ] **CI-4 真机联调**（待 Go 后端 + 真账号环境）：89 命令 × 8 阶段联调矩阵见 cses-client `docs/2026-06-23-helix-takeover-联调测试矩阵-CI4.md` + `2026-06-23-HTTP-WS接管联调方法论.md`
