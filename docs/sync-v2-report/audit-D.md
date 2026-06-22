# Phase D 全枚举审计 — #3 makeTopic + S4/S5/S6/S7/S9/S10 cross-check + #5 verify

> 📦 历史归档（2026-06-17 sync-v2 re-port 审计）：源分支 feat/dev-zlc 已合并入 main 并删除；本审计枚举的 A/B/C/D 各 phase 改动均已落地入 main。保留作审计留痕，**非活跑 tracker**。

> READ-ONLY 审计。来源真源 = `feat/sync-v2-full-replacement`（真机验证）；目标 = `dev-zlc`（ADR-007 架构）。
> diff 基线：`git diff dev-zlc...feat/sync-v2-full-replacement -- crates/helix-im`（helix-im +5209 / −162，60 文件）。
> SPEC 真源：`crates/helix-im/docs/csesapi-migration/SPEC-sync-v2-full-replacement.md`（515 行，feat 新增）。
> 落地真相：`crates/helix-im/docs/sync-v2-能力与缺口状态.md`（2026-06-18 真机闭环，§3 列 #3 makeTopic = DONE via replay）。

---

## 0. 一句话裁决

| 项 | 裁决 | 一句话 |
|---|---|---|
| **#3 makeTopic** | 🔴 **必须 re-port**（无独立模块，落在 S1+S3 channel-write 整层上）| dev-zlc 缺 `channel_write/` 整目录 + `channel_created`/`post_update` handler |
| **S4 写顺序** | 🟢 **已合规**（parser 分桶 + StorageOp 顺序天然 type1→2→3→6）；测试要随包补 | dev-zlc parser 已分桶；缺 `s4_write_order_test.rs` 钉死 |
| **S5 echo（3 级）** | 🟢 **已合规**（`pending_send.rs::reconcile` 已做覆写 send_status='sent'+id+CancelTimer）| 缺 `s5_post_echo_test.rs` 钉死真源 `post` 帧 |
| **S6 续拉/too_long** | 🟢 **已合规**（`SYNC_EVENTS_PER_ENTRY=500` 续拉 + `TooLong{reset_to}` 不删表）| sync_session.rs 仅 +4（viewers 字段）；缺 `s6_sync_internal_test.rs` |
| **S7 读契约（2D cursor）** | 🟢 **本质合规**（读旁路不接管，只需写表列对齐）；随 S3 落表自动满足 | 无独立 re-port；验收靠 S3 channel/message 列对齐 |
| **S9 emit 全集** | 🔴 **必须 re-port**（缺 read_echo 分流 + posts_update 批量 + S1 channel/schedule emit）| 含 A4/A5 两处 emit 命名/形态修复 |
| **S10 命名陷阱** | 🟡 **部分**（既有契约保留；S1/S3 新增字段的 rename 随 re-port 一起带）| 无独立动作，随 S1/S3/S9 落地 |
| **#5 auth** | ✅ **已满足，不 re-port** | dev-zlc `helix-host-cli/src/main.rs:135` `set_global_header("cookieId", …)` 全局头注入；搬进 ImConfig 违 ADR-010 |

---

## 1. #3 makeTopic — re-port checklist

### feat 怎么实现 makeTopic（建话题 / channel topic 创建）

makeTopic **没有独立 handler**——它复用 S1+S3 的两条既有路径，靠注释显式标注（grep 实证）：

- `src/ws/handlers/channel_created.rs:4` — *"建话题 makeTopic 也发 channel_created"* → path1 全 52 列 upsert → emit `im:channel:created`。
- `src/ws/handlers/post_update.rs:8` — *"建话题 makeTopic 也走 post_update（spec §S1）"*。

即：**建话题 = 一次 `channel_created` 帧（52 列 channel upsert）+ 后续 `post_update` 流**。dev-zlc 当前 **两个 handler 都不存在**，且 `channel_write/` 整目录缺失（`ls` 实证 No such file or directory），故 makeTopic 必须随 S1+S3 一起 re-port。

### re-port 清单（目标路径 — 在 `worktrees/sync-v2-report/crates/helix-im/`）

| # | 搬什么 | feat 源路径 | dev-zlc 目标路径 | 行数 |
|---|---|---|---|---|
| 1 | channel-write 整目录（path1/2/3 程序 + 成员 + patch） | `src/channel_write/{mod,program,members,patch,post}.rs` | 同路径（新建 dir） | mod 81 / program 231 / members 217 / patch 165 / post 257 |
| 2 | `pub mod channel_write;` | `src/lib.rs` (+1) | `src/lib.rs` | 1 |
| 3 | channel 三路径 ACL 工厂 | `src/acl/to_effect.rs` (+224，见 §3) | 同 | 224 |
| 4 | S1 新增 emit/storage 工厂 | `src/acl/to_effect_s1.rs`（**NEW 183**） | 新建 | 183 |
| 5 | `pub mod to_effect_s1;` | `src/acl/mod.rs` (+1) | `src/acl/mod.rs` | 1 |
| 6 | channel_created handler（makeTopic emit 路径） | `src/ws/handlers/channel_created.rs` (63) | 新建 | 63 |
| 7 | post_update handler（makeTopic post 流） | `src/ws/handlers/post_update.rs` (94) | 新建 | 94 |
| 8 | 其余 6 S1 handler（见 §S9） | `src/ws/handlers/{channel_close,channel_member_update,post_read,posts_update,update_channel,update_channel_member_nickname,post_schedule_created,post_schedule_canceled}.rs` | 新建 | 60–119 各 |
| 9 | handler registry 注册 14 action | `src/ws/handlers/mod.rs` (+11) / `src/ws/registry.rs` (+30) | 同 | — |
| 10 | `sync_session.rs` `PostFields.viewers` 字段（A3 可见性谓词） | `src/sync_session.rs` (+4) | 同 | 4 |
| 11 | `parser.rs` (+52) / `channel.rs` (+106) / `module.rs` (+113) / `schema.rs` (+25) / `state.rs` (+2) / `pending_send.rs` (+2) / `ws/context.rs` (+5) | 同名文件 delta | 同 | — |

**验收锚**：feat `tests/channel_write_program_test.rs`(138) + `channel_member_collect_test.rs`(196) + `s1_ws_actions_test.rs`(576) 一并搬。makeTopic 端到端 = 发 `channel_created` 帧 → 断言 52 列 channel upsert + `im:channel:created` emit。

---

## 2. S4/S5/S6/S7/S9/S10 逐项裁决

### S4 — message 表写顺序铁律 🟢 已合规（缺测试）

**铁律**（SPEC §S4 / 测试 `s4_write_order_test.rs:64-129`）：同批落库 StorageOp 必须严格 **type1(upsert)→type2(edit)→type3(revoke)→type6(read) 最后**。否则 type1 全量 upsert 经 `ON CONFLICT(temporary_id) DO UPDATE` 会用 map body 的旧 read_bits **擦掉** type6 单调覆盖的权威 read_bits。

- **dev-zlc 现状**：parser 已分桶（`parser.rs:374-377` EventKind 映射 type1/2/3/6；`type_num` 排序键 `parser.rs:388-392`）；`batch_upsert_events_with_messages` 按桶序产 op。**机制已对齐**。
- **db_ok 才推 cursor**：dev-zlc gate 三阶段 peek→persist→commit，commit 依赖 persist corr 回调成功（gate 铁律，CLAUDE.md 不变量 5）——`s4_persist_err_does_not_advance_cursor`（test:131-183）证 persist Err → cursor 不推进、re-pull。dev-zlc 已有此语义。
- **缺**：`tests/s4_write_order_test.rs`(183) 没搬——补它把「type1 在 type6 前」「四桶严格序」「persist err 不推 cursor」三条钉死防漂移。
- **裁决**：**已合规，只补测试**（随 S4 测试文件搬）。

### S5 — echo 对账（3 级查找）🟢 已合规（缺测试）

**契约**（SPEC §S5）：自发回流走**同一 `"post"` action**（无独立 echo type），靠 `temporaryId` 非空 + `user_id==auth_user_id` 区分；三级查找 post_id→temporary_id→post_id-as-tmp；命中 → `update_partial` 按 temporary_id 主键回填 server id + send_status='sent' + CancelTimer 撤 15s 超时。

- **dev-zlc 现状**：`pending_send.rs::reconcile`(:46-78) 已做 **① Persist ON CONFLICT(temporary_id) 覆写 id=serverId+status=sent ② CancelTimer 撤 15s**（pending_send.rs:5/11/44-46 注释 + 代码实证）。`ws/handlers/post.rs:43-47` 已做 echo-gate 解耦（reconcile 只读 pending_sends/corr_map，temporary_id 命中在途 send 才对账）。**核心机制已对齐**。
- **分层契约**：core 无身份 → echo 保守产 unread bump=1（sender 豁免下沉 driver），feat `s5_post_echo_test.rs:116-134` 钉死。dev-zlc 同策略（无身份）。
- **缺**：`tests/s5_post_echo_test.rs`(162) 没搬——它用**真源 `post` 帧**（非 `posted`）锁定 echo→reconcile+CancelTimer+cursor 推进 + 无 temporary_id（他人推送）非 echo 不动 pending_send。
- **裁决**：**已合规，补测试**。注意 feat `post` handler 在对账后**移除** pending_send（防 corr_map 泄漏，与 `posted` 保留态不同，test:66-67 注释）——若 dev-zlc post 帧路径要随 S1 搬，核对这点。

### S6 — sync 引擎内化 / 续拉 / too_long 🟢 已合规（缺测试）

**契约**（SPEC §S6 / HX-C008）：① `events.len() >= 500` → 续拉至 no_change（真 wire 无显式续拉 flag，满批即视为还有更多）；② `too_long`(gap>5000) → Rust 只 `cursor=reset_to-1` + buffer.clear + emit `im:sync:too_long`，**Rust 侧无 message DELETE**（清表交前端）；③ increment_end fan-out 对 (fetched−skip) 确定性有序（HX-C010）。

- **dev-zlc 现状**：
  - 续拉：`parser.rs:198 SYNC_EVENTS_PER_ENTRY=500` + `:264` 满批续拉注释 + `sync_session.rs:7-8/140` `needs_continuation`（实证）。
  - too_long：`sync_session.rs:158 TooLong{reset_to}` + `:8` 注释「cursor=reset_to-1 + Emit(SyncTooLong) → 前端清表重拉」；`StorageOp` 枚举无 Delete 变体（类型层即排除 Rust 删消息）。
  - **diff 实证**：`sync_session.rs` feat vs dev-zlc **仅 +4 行**（新增 `PostFields.viewers` 可见性受众字段，A3/CAP-9 未读 +1 谓词用），sync 主体逻辑两边一致。
- **缺**：`tests/s6_sync_internal_test.rs`(141)——钉死 ① too_long emit 但不写/删 message 表 ② increment_end fan-out 按 ChannelId 升序确定性。
- **裁决**：**已合规**。唯一需搬的源码 = `PostFields.viewers` 字段（+4，随 S3/A3 未读链一起，见 §3 `sync_unread_bump_ops`）+ 该测试。

### S7 — 读旁路契约（2D cursor）🟢 本质合规（无独立 re-port）

**契约**（SPEC §S7）：前端读消息走 `store_get_messages` 三分支（首屏/上拉/下拉），**CAP-4 二维游标硬契约** = 排序键 `(create_at, temporary_id)` 词典序（**非单维 create_at**，同毫秒 >20 条单维会 spin-loop）；helix 不接管读，但**写出来的表必须满足读契约**（message 表含 `create_at`+`temporary_id` 列；channel 列名 == store_query 白名单 camelCase→snake_case）。

- **裁决**：**无独立 re-port 动作**。S7 是「写表对齐读契约」的约束，随 S3 channel 52 列 upsert（`channel_write/program.rs`）+ message type1 upsert（PK=temporary_id）落地后自动满足。验收 = 替换后前端三分支读分页无 spin-loop（同毫秒 >20 条用例）。注意 message 表 PK = `temporary_id`、二维游标第二键也是 `temporary_id`（feat 已保证）。

### S9 — emit 总线全集对齐 🔴 必须 re-port

emit 全集是 `to_effect.rs` +224 与 `to_effect_s1.rs`(NEW 183) 的核心载荷，含**两处真机定位的命名/形态修复**：

- **A5 修复**（`to_effect.rs` delta）：type6 read 分两条 —— **在线** WS post_read 发 `im:post:read`（`emit_post_read`，真源 post.rs:157/167）；**离线 sync 应用**的 type6 发 `im:channel:read_echo`（**新增** `emit_channel_read_echo`，真源 post.rs:1428）。此前 helix `sync_mutation_emits` 统一发 `im:post:read` → 违 spec §0.3/§S9 前端零改动契约（前端 read_echo 订阅分支收不到）。**dev-zlc 当前统一命名 = 有 bug，必须分流**。
- **A4 修复**（`to_effect.rs` delta）：`posts_update`（批量编辑/撤回）→ **新增** `emit_post_batch_updated`，事件名 `im:post:batch-updated` + payload `{posts:[...]}`（前端 message_loop.rs:187/341 按此名+数组解析）。此前误用 `emit_post_updated`（名+形态双错）。
- **S1 新增 emit**（`to_effect_s1.rs`）：`im:channel:created` / `im:channel:closed` / `im:channel:member-updated` / `im:channel:memberNickname` / `im:channel:schedule-created` / `im:channel:schedule-canceled`（各 emit_* 工厂，真源 channel.rs:569/593/626/757 + schedule.rs:25/49）。
- **裁决**：**必须 re-port**——`to_effect.rs` +224（含 read_echo/batch_updated/channel-write 工厂）+ `to_effect_s1.rs` 全文。emit channel 名全集 grep 对照现网（SPEC §S9 验收）。

### S10 — 命名陷阱契约 🟡 部分（随 S1/S3/S9 带）

逐字保留的 rename（SPEC §S10 表）：`unReadPostId`（大写 R）/ `temporaryId`（echo 键）/ `readBits` / `createAt`（少 d）/ `SyncEntryKind`（唯一 snake_case）/ 外层信封 `status/message/data` 不 rename / event_type 数值 `1/2/3/6`（无 4/5）/ 阈值 too_long>5000、续拉 500、getLatestPost 20。

- **dev-zlc 现状**：既有契约（temporaryId/readBits/createAt/SyncEntryKind/500/too_long）**已合规**（parser.rs:198/212/258-284 + pending_send temporary_id 实证）。
- **缺**：S3 channel 52 列的 JSON rename（`displayName`/`teamId`/`channelIsTop`/`unreadCount`/`unreadPostId`/`mentionList`/`lastPostAt`/`memberCount` 等，见 `channel_write/program.rs` 每列）+ S1 字段 rename，随 §1 re-port 自动带入。
- **裁决**：**无独立动作**，随 S1+S3+S9 落地一并满足；验收靠 golden fixture 真帧解析无字段丢失。

---

## 3. acl/to_effect_s1.rs + to_effect.rs delta — 要 port 的 emit/naming 逻辑

### to_effect_s1.rs（NEW 183 行，全文 port）

S1 新增 WS action 的 Effect 工厂（与 to_effect.rs 分文件守 ≤300 行硬顶）。职责 = IM 概念→core 通用 Effect/StorageOp，表名/列名/`im:` 前缀作**数据**经 spec 传给 core（守 HX-C001）：

| 工厂 | 产出 | 真源 |
|---|---|---|
| `emit_channel_created(cid, raw)` | Emit `im:channel:created`（透传帧 raw 字节） | channel.rs:569 |
| `emit_channel_closed(cid, delete_at)` | Emit `im:channel:closed{channelId,deleteAt}` | channel.rs:593 |
| `emit_channel_member_updated(cid, raw)` | Emit `im:channel:member-updated` | channel.rs:626 |
| `emit_member_nickname(cid, uid, nick)` | Emit `im:channel:memberNickname` | channel.rs:757 |
| `emit_schedule_created/canceled(cid)` | Emit `im:channel:schedule-{created,canceled}{hasSchedulePost}` | schedule.rs:25/49 |
| `channel_set_cols(cid, cols)` | `PersistFire` BatchUpdate channel SET cols WHERE id=?（O(1)，channel_close/schedule 定点列） | — |
| `upsert_channel_members(cid, members)` | `PersistFire` BatchUpsert channel_member ON CONFLICT(channel_id,user_id)（memberChange.join，空集→None） | — |
| `update_member_nickname(cid, uid, nick)` | 复合 PK upsert（`exclude_from_update:[team_id,role]` → 冲突仅改 nick_name；缺行插占位）| channel_member_repo::update_nickname |

> 关键坑（注释 :150-161）：core `BatchUpdate` 仅 `WHERE key_col IN(...)`（单列），**无法**表达复合 `WHERE channel_id=? AND user_id=?` → 用复合 PK upsert 代替，否则误改该 channel 全部成员 nick_name。

### to_effect.rs delta（+224 行，port）

| 新增/改 | 内容 | 真源 / 修复标记 |
|---|---|---|
| `upsert_channel_full(cols, exclude)` | S3 path1：channel 全 52 列 upsert ON CONFLICT(id)，exclude=守卫/本地列保留既有值 | — |
| `update_channel_partial(cid, cols)` | S3 path2：channel PATCH UPDATE SET cols WHERE id=?（空 cols→None） | — |
| `bump_channel_unread(upd)` / `bump_channel_unread_op(upd)` | S3 path3：未读 **SQL 自增** `GuardedBump`（守 HX-C005 禁 RMW），守卫 `msg_create_at>last_root_post_at` 同门控 lastPost 组+未读组；delta==0（自己/不可见）仍写 lastPost | — |
| `advance_cursor` 改表 | S2：目标表 = `channel_event_cursor`（byte 对齐现网 678），`MonotonicUpsert{table,key_col:channel_id,value_col:last_event_seq,touch_col:updated_at}` | tables/channel_event_cursor.rs:49 |
| `emit_channel_read_echo(cid,seq,msg)` | **A5 新增**：离线 sync type6 → `im:channel:read_echo`（与在线 `im:post:read` 分流） | post.rs:1428 |
| `emit_post_batch_updated(cid,posts)` | **A4 新增**：posts_update → `im:post:batch-updated{posts:[...]}` | post.rs:254 |
| `sync_mutation_emits` 改 | type6 分支 `emit_post_read`→`emit_channel_read_echo`（A5 分流） | — |
| `sync_unread_bump_ops(events,messages,auth_user_id)` | **A3/CAP-9 新增**：离线 sync 可见 type1 → channel 未读 +1 **落 DB**（不只 emit）；phantom 不 bump；与在线 bump 同口径互斥不重复 +1 | post.rs:1234 |
| `upsert_message` | +`exclude_from_update: Vec::new()`（UpsertSpec 字段补全） | — |

> `sync_unread_bump_ops` 依赖 `PostFields.viewers`（sync_session.rs +4）+ `channel_write::post_updates_from_fields`——三者一起 port。

---

## 4. #5 auth — VERIFY 结果

**裁决：✅ 已满足，不 re-port。**

**证据**（dev-zlc `crates/helix-host-cli/src/main.rs`）：
- `:53` `let cookie_id = std::env::var("HELIX_COOKIE_ID")` — 身份从 host env 注入。
- `:132-136` `http.set_global_header("cookieId", &cookie_id)` — **cookieId(=userId) 注入为 driver 全局 HTTP 身份头**（现网 sync-v2 鉴权两件套之一；注释「A3a」）。
- `:144/151/158` 同路注入 `companyId`/`appType`/`deviceId` 全局头。
- `:189-190` WS 侧 `ws_headers.push(("cookieId", cookie_id))` — 现网 Go 在 WS 升级阶段按 cookieId=userId 建 session。
- driver 层 `helix-driver-native/src/http.rs:28 set_global_header` + `helix-driver-ffi/src/api.rs:228 helix_set_header` — 全局头机制三端就位。

**结论**：鉴权由 **driver 全局头（ADR-010 SharedHttpClient）** 统一覆盖，对所有 HTTP/WS 出站生效。把鉴权搬进 `ImConfig` 会让 helix-im 业务层持有身份 = **违 ADR-010 + HX-C001（core/im 零身份）**。#5 **不搬，仅核实通过**（与 SESSION.md:18/27 一致）。

---

## 5. 流程（mermaid 用 — 编号列表）

### 5.1 emit/effect flow（to_effect mapping — sync 应用路径）

1. WS 入站帧 / sync 响应 → `parser::parse_sync_response` → `(events: Vec<EventEnvelope>, messages: HashMap<msg_id, PostFields>)`。
2. parser 分桶稳定排序 → events 序列 type1→type2→type3→type6（S4 铁律）。
3. 落库 op 生产 `batch_upsert_events_with_messages(events, messages)`：
   3.1 type1 PostUpsert → `BatchUpsert(message, ON CONFLICT temporary_id)`。
   3.2 type2 PostEdit → `BatchUpdate(message, patch=content)`。
   3.3 type3 PostRevoke → `BatchUpdate(message, patch=revoke)`。
   3.4 type6 PostRead → `BatchUpdate(message, patch=read_bits)` **殿后**（防擦）。
4. 未读落库 `sync_unread_bump_ops(events, messages, auth_user_id)`：可见 type1（命中 messages map）→ `GuardedBump(channel.unread_count += delta WHERE id=? AND msg_create_at>last_root_post_at)`；phantom 跳过。
5. emit 生产 `sync_mutation_emits(events, messages)`：
   5.1 type1 可见 → `im:post:received`；type1 phantom → 不 emit（cursor 照推）。
   5.2 type2 可见 → `im:post:updated`；phantom → noop。
   5.3 type3 → `im:post:deleted`（有 msg_id 即 emit）。
   5.4 type6 → `im:channel:read_echo`（**A5 离线分流**，非在线 `im:post:read`）。
   5.5 posts_update 批量 → `im:post:batch-updated{posts:[...]}`（A4）。
6. 所有 StorageOp 并入**同一** `Effect::Persist{corr}` 批（CAP-9：DB 写成功后 cursor 才推进）。
7. persist corr 回调成功（db_ok）→ gate `commit_up_to` 推进 cursor + `advance_cursor` 写 `channel_event_cursor` 表；db_ok 失败 → cursor 原地不动 → 下次 re-pull（S4）。
8. Effect 批整批交 driver 解释（HX-C007 跨边界批处理）；driver `NativeEventSink::emit` → `app.emit("im:__bus__", envelope)`（M2 接缝）。

### 5.2 sync 续拉 / too_long flow（S6 / HX-C008）

1. gate fire（timer）→ `sync_notify` Effect → HTTP `POST /api/cses/channel/sync/notify` body `{cursors:[{channelId, fromSeq}]}`。
2. HTTP corr 回调 → `parse_sync_response` → 取本 channel 的 `SyncEntryKind`（snake_case）。
3. **no_change**（gap0）→ 推 next_seq（跳合法稀疏空洞，不卡空洞防 sync 风暴）→ 结束。
4. **events**（0<gap≤5000）→ 落库（§5.1 流程）+ commit cursor。
   4.1 判 `events.len() >= 500`（`SYNC_EVENTS_PER_ENTRY`）？ 真 wire 无显式 flag → 满批即视为「还有更多」。
   4.2 是 → 续拉（用新 cursor 再发 sync/notify，回步骤 1）直到 `len<500` 或 no_change（UC-3：cursor==服务端 maxSeq）。
   4.3 否 → 已追平，结束。
5. **too_long**（gap>5000）→ ① cursor = `reset_to - 1` ② buffer.clear ③ emit `im:sync:too_long{channelId, resetTo}`。**Rust 侧零 message DELETE**（StorageOp 无 Delete 变体）；前端 `onRustTooLong` 清表 + `getLatestPost` 重拉首屏 + dispatch POST_INCREMENT。
6. **snapshot**（A2 reserved）→ 未实现，占位。
7. `increment_channel_end` → `proactive_resync_all_channels(increment_fetched − need_sync_skip)`：迭代前 **sort（HX-C010 确定性有序）** → 逐 channel 发 sync/notify，BoundedSpawner 限流（`HELIX_HTTP_MAX_INFLIGHT`，防冷启动 N channel 并发风暴）。

---

## 6. 验收测试（名 + 断言）

### S4 — `tests/s4_write_order_test.rs`（183，搬）

- `s4_type1_before_type6_in_same_batch`：wire 乱序投 type6 在前 → parser 分桶纠正 `[1,6]`；落库 op `position(type1) < position(type6)`（否则擦 read_bits）。
- `s4_four_bucket_strict_order`：四类混批 → ops 严格 `[Type1Upsert, Type2Edit, Type3Revoke, Type6Read]`。
- `s4_persist_err_does_not_advance_cursor`：persist `PortError::Storage` → `cursor_for(ch)==Some(0)`（写库成功才 commit）。

### S5 — `tests/s5_post_echo_test.rs`（162，搬）

- `s5_post_action_echo_reconciles`：发送→PendingSend(Local)+15s timer；`post` 帧（temporary_id 命中，seq=1）→ message Persist `conflict_key=temporary_id` + `send_status='sent'` + `id=server_id(42)` + CancelTimer(15s) + cursor→1。
- `s5_core_echo_conservatively_bumps_unread_sender_exemption_deferred`：core 无身份 → echo 仍产 `GuardedBump(unread_count, delta=1)`（sender 豁免下沉 driver）。
- `s5_post_action_without_temp_id_is_not_echo`：`post` 帧无 temporary_id（他人推送）→ pending_send 仍 Local + 无 CancelTimer。

### S6 — `tests/s6_sync_internal_test.rs`（141，搬）

- `s6_too_long_emits_but_does_not_delete_message`：too_long(resetTo=100) → ① emit `im:sync:too_long` ② 遍历所有 Persist/PersistFire op 断言**无任何 message 表写**（BatchUpsert/Update/GuardedBump/Monotonic/Scan/Get 的 table 均≠"message"）③ cursor 复位 `reset_to-1=99`。
- `s6_increment_end_fanout_deterministic_order`：乱序 ingest(30,2,17) → `ingest_increment_end(None)` → fan-out sync/notify 的 `cursors[0].channelId` 顺序 == `[cid(2),cid(17),cid(30)]`（ChannelId 升序，HX-C010）。

### makeTopic + S1/S3（搬，随 channel-write）

- `tests/channel_write_program_test.rs`(138)：`program_full` 52 列 upsert 行 + JSON rename + 守卫 omit 集断言。
- `tests/channel_member_collect_test.rs`(196)：memberChange.join 收集 + 复合 PK upsert。
- `tests/s1_ws_actions_test.rs`(576)：14 WS action 全集（含 channel_created=makeTopic 路径、post_update、posts_update、post_read、update_channel、channel_close、member_update、nickname、schedule×2）逐 action 回放断言 emit 名 + StorageOp 形态。
- makeTopic 端到端断言：`channel_created` 帧 → 52 列 channel upsert（ON CONFLICT id）+ emit `im:channel:created`。

---

## 7. 数据出处（file:line 索引）

- makeTopic 标注：feat `ws/handlers/channel_created.rs:4`、`ws/handlers/post_update.rs:8`（grep 实证）。
- dev-zlc 缺口：`ls channel_write/` = No such file or directory；`ls ws/handlers/` 仅 8 文件 vs feat 20 文件。
- #5 auth：dev-zlc `helix-host-cli/src/main.rs:53/132-136/189-190`；driver `helix-driver-native/src/http.rs:28`、`helix-driver-ffi/src/api.rs:228`。
- S4：feat `s4_write_order_test.rs:64-183`；dev-zlc `parser.rs:374-392`。
- S5：dev-zlc `pending_send.rs:44-78`、`ws/handlers/post.rs:43-47`；feat `s5_post_echo_test.rs`。
- S6：dev-zlc `parser.rs:196-284`、`sync_session.rs:7-165`（feat sync_session.rs 仅 +4 viewers）；feat `s6_sync_internal_test.rs`。
- emit delta：`git diff dev-zlc...feat -- crates/helix-im/src/acl/to_effect.rs`（+224）、`to_effect_s1.rs`（NEW 183）。
- 落地真相：`docs/sync-v2-能力与缺口状态.md` §3（#3 makeTopic=DONE replay / #5 auth=DONE 30bc9a8）。
