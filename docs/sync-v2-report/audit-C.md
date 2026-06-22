# Audit C — Phase C: 缺失 WS action handler（S1 全 14-action 对齐）

> 📦 历史归档（2026-06-17 sync-v2 re-port 审计）：源分支 feat/dev-zlc 已合并入 main 并删除；本审计枚举的 A/B/C/D 各 phase 改动均已落地入 main。保留作审计留痕，**非活跑 tracker**。

> READ-ONLY 枚举审计。源真值 = `worktrees/sync-v2-full-align`（`feat/sync-v2-full-replacement`，真机验证）。
> 目标 = `worktrees/sync-v2-report`（`dev-zlc`，ADR-007 架构）。
> 对比命令：`git diff dev-zlc...feat/sync-v2-full-replacement -- <path>`（repo root）。
>
> **关键机制**：handler 注册经 `linkme::distributed_slice(WS_HANDLERS)` —— **无手工注册表数组**。
> 每个 handler 文件自带一个 `#[distributed_slice(WS_HANDLERS)] static …_REGISTRATION` 块，
> 链接期自动汇入 `WS_HANDLERS` 切片，`registry.rs` 启动时一次性构建 action→handler 的 `HashMap`（O(1) 分发）。
> 所以 **registry.rs 本身不需要逐 action 改**——「wiring」= 在 `ws/handlers/mod.rs` 加 `pub(crate) mod` 声明
> 把新文件拉进编译单元（否则 `distributed_slice` 块不参与链接）。`registry.rs` 的 +30 行 diff 全是
> 测试 call-site 适配 `ImWsContext::new` 新增的 `auth_user_id` 参数（非分发逻辑变更）。

---

## 1. 全 14 WS action 总表

| # | action 名 | handler 文件 | 入站 payload（关键字段） | 吐出 Effect | 已在 dev-zlc? | 依赖 Phase B(channel_write)? |
|---|---|---|---|---|:--:|:--:|
| 1 | `post`（type=1 新消息）| `post.rs`（**改**，+84）| `data.post` 或顶层：`id`/`channelId`/`message`/`event_seq`/`temporaryId`/`userId`/`viewers` | gate ingest（Persist message + cursor）+ **path3 `bump_channel_unread`**（未读 +1 SQL 自增）+ emit | Y（基础）/ N（path3 增量）| **Y**（path3）|
| 2 | `posted`（echo 对账）| `posted.rs` | `data`：`temporaryId`/`channelId`/`seq` | 发送对账 reconcile + cursor | Y | N |
| 3 | `increment_channel`（path1 全量）| `increment_channel.rs`（**改**，+40）| `id`/`lastEventSeq`(Optional)/`needSync`/`members`/`adminUsers`/`boss`/`owner`/`memberChange.join` | **`upsert_channel_full`(52列)** + **`upsert_channel_members`** + emit increment + 种 increment_target | Y（cursor/emit）/ N（path1 + 成员落库）| **Y** |
| 4 | `increment_channel_end`（冷启动 heal）| `increment_channel_end.rs`（**改**，+7）| 无 data（信号帧）| B2 heal：对 cursor<target 的 channel 补发 sync（`stuck.sort_unstable()` HX-C010 有序）| Y / 部分（缺 sort + path1 联动）| 间接 |
| 5 | `post_update`（type=2 单条编辑）| **`post_update.rs`（新）** | `data` 即 post：`id`/`channelId`/`message`/`props`/`event_seq` | gate ingest(PostEdit→`edit_content_op` 仅内容列 UPDATE，**不擦 read_bits**)+ emit `im:post:updated`+cursor | **N** | N（用 gate + parser）|
| 6 | `posts_update`（type=2/3 批量）| **`posts_update.rs`（新）** | `data.posts:[Post]`（per-post `revoke`/`id`/`message`）+ 顶层 `event_seq` | 逐 post `PersistFire`（revoke→`revoke_op` / else `edit_content_op`）+ `gate_advance_cursor_only` + emit `im:post:batch-updated` | **N** | N（用 gate + parser）|
| 7 | `post_read`（type=6 已读覆盖）| **`post_read.rs`（新）** | `postId`/`readMap`(→read_bits)/`channelId`/`event_seq` | gate ingest(PostRead→覆盖式 read_bits UPDATE WHERE id)+ emit `im:post:read`+cursor | **N** | N（用 gate + parser）|
| 8 | `channel_created`（path1 + 成员）| **`channel_created.rs`（新）** | `id`/`type`/`displayName`/`memberChange.join`/`owner`/`members` | `program_full`(52列 upsert)+`upsert_channel_members`+emit `im:channel:created`（透传帧）| **N** | **Y** |
| 9 | `channel_member_update`（path1 + 成员）| **`channel_member_update.rs`（新）** | `id`/`type`/`lastEventSeq`/`memberChange.join`（role 经 normalize MANAGER→ADMIN）| `program_full`+`upsert_channel_members`+emit `im:channel:member-updated` + 种 increment_target（cursor 种子 0，HX-C008）| **N** | **Y** |
| 10 | `update_channel`（path2 PATCH）| **`update_channel.rs`（新）** | `id`/`channelId` + 任意 `Some` 列（白名单）| `collect_present`→`update_channel_partial`(UPDATE … WHERE id；空集→无写)| **N** | **Y** |
| 11 | `update_channel_member_nickName`（注意驼峰混 snake）| **`update_channel_member_nickname.rs`（新）** | `channelId`/`userId`/`nickName` | `update_member_nickname`(复合 PK upsert，conflict_key=`channel_id,user_id`)+emit `im:channel:memberNickname` | **N** | N（用 to_effect_s1）|
| 12 | `post_schedule_created` | **`post_schedule_created.rs`（新）** | `channelId` | `channel_set_cols(has_schedule_post=1)`+emit `im:channel:schedule-created` | **N** | N（用 to_effect_s1）|
| 13 | `post_schedule_canceled` | **`post_schedule_canceled.rs`（新）** | `channelId` | `channel_set_cols(has_schedule_post=0)`+emit `im:channel:schedule-canceled` | **N** | N（用 to_effect_s1）|
| 14 | `gate`（**非 action**，共享 helper）| **`gate.rs`（新）** | — | `gate_ingest_content_event`/`gate_advance_cursor_only`：把带 event_seq 的内容帧经 channel gate（严格 +1 / drop / buffer+1s）| **N** | N（被 5/6/7 依赖）|

> 共 13 个真 action + 1 个共享 helper(`gate.rs`)。dev-zlc 缺 8 个新 action 文件 + `gate.rs`；并需改 `post.rs`/`increment_channel.rs`/`increment_channel_end.rs` 3 个既有 handler。

---

## 2. 每个 NEW handler 的目标路径 + 一段式 spec

所有路径相对 `crates/helix-im/src/ws/handlers/`。

- **`gate.rs`**（先建，5/6/7 的前置）：共享 helper，非 action。`gate_ingest_content_event(ctx, frame, channel_id, kind, fields, msg_id, out)` 把带 `event_seq` 的内容帧（type2/3/6）经 `Channel::ingest`（严格 +1 / drop / buffer+arm 1s gate，绝不 MAX-jump 跨空洞，守 §S6）；`gate_advance_cursor_only(ctx, frame, channel_id, out)` 仅推 cursor（`posts_update` 批量帧用，`Channel::ingest_cursor_advance`）。无 event_seq / 未注册 channel → no-op（边界零信任）。落库 + emit 由 gate kind-aware 完成。

- **`post_update.rs`**（action `post_update`，type=2）：解析 `data`（即单 post），`channelId` 硬 gate，`extract_post_fields` 抽 fields，`gate_ingest_content_event(EventKind::PostEdit, msg_id=data.id)`。gate 用 `edit_content_op`（仅内容列 UPDATE WHERE id，保留本地 read_bits/send_status）+ emit `im:post:updated`。建话题 makeTopic 也走此路径。

- **`posts_update.rs`**（action `posts_update`，批量 type=2/3）：`data.posts` 数组缺/非数组→noop。逐 post：`revoke==true`→`channel::revoke_op`，else→`channel::edit_content_op`，各包 `Effect::PersistFire`。channel_id 取首 post 或顶层兜底。`gate_advance_cursor_only`(顶层 event_seq 推一次) + emit `im:post:batch-updated`（payload `{posts:[...]}`，**非** `im:post:updated`）。

- **`post_read.rs`**（action `post_read`，type=6）：`postId`→msg_id（WHERE 键），`channelId` 硬 gate。fields = `extract_post_fields`，若 read_bits 空则取 `readMap`/`read_map` 覆盖。`gate_ingest_content_event(EventKind::PostRead, msg_id=postId)`：gate 覆盖式写 read_bits（服务端权威单调，非按位 OR）+ emit `im:post:read`。缺 postId → 不写 message（但 channel 仍可推 cursor）。

- **`channel_created.rs`**（action `channel_created`，path1）：`id` 硬 gate（非 26 字符→整体 noop）。委托 `channel_member_update::apply_channel_full_and_members`（种 cursor=0 / 种 increment_target / `program_full` 52 列 upsert / `collect_members` 四源+join 写 channel_member）+ emit `im:channel:created`（透传帧 raw，含 owner/members/picture）。

- **`channel_member_update.rs`**（action `channel_member_update`，path1 + 成员；**也导出共享 `apply_channel_full_and_members`**）：`id` 硬 gate。`apply_channel_full_and_members`：①`or_insert_with(Channel::new(id, 0))` 种 cursor 0（**HX-C008**：绝不种 lastEventSeq）②`lastEventSeq`→`increment_target`（服务端水位，与 cursor 解耦）③`program_full(data, ctx.auth_user_id, ctx.now_ms)`→`upsert_channel_full` ④`collect_members`(四源 members/adminUsers/boss/owner + memberChange.join，role normalize)→`upsert_channel_members`。emit `im:channel:member-updated`。

- **`update_channel.rs`**（action `update_channel`，path2 PATCH）：`id`/`channelId`/`channel_id` 硬 gate。`collect_present(data)`（只收 `Some` 字段，白名单过滤）→`update_channel_partial(id, cols)`（空集返回 None→不 push）。成员/owner/admin/boss 不走此路径。

- **`update_channel_member_nickname.rs`**（action 字面量 **`update_channel_member_nickName`**，驼峰混 snake，**原样保留**）：`channelId`/`userId`/`nickName` 三键，缺 channelId 或 userId → noop。`update_member_nickname(id, user_id, nick)`（复合 PK upsert，conflict_key `channel_id,user_id`，冲突仅改 nick_name）+ emit `im:channel:memberNickname`。

- **`post_schedule_created.rs`**（action `post_schedule_created`）：`channelId` 硬 gate。`channel_set_cols(id, [(has_schedule_post, Integer(1))])`（定点列 patch UPDATE WHERE id）+ emit `im:channel:schedule-created`。

- **`post_schedule_canceled.rs`**（action `post_schedule_canceled`）：同上但 `has_schedule_post=0` + emit `im:channel:schedule-canceled`。

- **`channel_close.rs`**（action `channel_close`）：`channelId` 硬 gate，`deleteAt` 取 i64 默认 0。`channel_set_cols(id, [(delete_at, deleteAt), (is_active, 0)])` + emit `im:channel:closed{channelId, deleteAt}`。本地无此行→0 行命中 no-op（幂等）。

> 注：表第 14 项把 `channel_close` 漏到 helper 槽——实际 NEW action 文件共 9 个（`channel_close` 是第 9 个 action 文件），加 `gate.rs` 共 10 个新文件。

---

## 3. parser.rs 新增 + registry wiring（编号）

### 3.1 parser.rs（diff +52，**非新 action 变体**，而是字段/容错增强）

1. `extract_post_fields`：`PostFields` 新增字段 **`viewers: Vec<String>`**（取 post 子树或 root 顶层的字符串数组，元素非字符串跳过，缺省空 Vec）。→ **依赖 `sync_session.rs::PostFields` 先加 `pub viewers: Vec<String>` 字段**（+4 行，A3/CAP-9 可见性谓词，post.rs path3 用）。
2. `parse_increment_channel`：`lastEventSeq` 从必填改 **Optional**（`.unwrap_or(0)`，不再 `?` 早退）。真 Go 为 `Option<i64>`（legacy mattermost 9.7.3 GetMaxSeq 降级场景缺此字段）。**仅 `id` 非法才返回 None。**（P0：实测 650 increment 帧曾因 `?` 早退被全静默丢。）
3. `parse_sync_response`：camelCase/snake_case 双形态容错——entry 匹配键 `channelId||channel_id`；`nextSeq||next_seq`；`resetTo||reset_to`（too_long + snapshot 两处）。

### 3.2 registry / context / mod wiring（编号）

1. **`ws/context.rs`**（+5）：`ImWsContext` 加字段 `pub(crate) auth_user_id: &'a str`（host 注入，与 `api_base_url` 同机制）；`new()` 增第 4 参 `auth_user_id`。→ path3 sender 豁免 + path1 channel `user_id` 列（BLOCKING-1）。
2. **`ws/handlers/mod.rs`**（+11）：加 10 个 `pub(crate) mod` 声明（`channel_close`/`channel_created`/`channel_member_update`/`gate`/`post_read`/`post_schedule_canceled`/`post_schedule_created`/`post_update`/`posts_update`/`update_channel`/`update_channel_member_nickname`）。**这是唯一必需的注册 wiring**——linkme 自动汇集。
3. **`ws/registry.rs`**（+30）：无分发逻辑改动；仅所有测试 call-site 把 `ImWsContext::new(state, now, url, &mut corr)` → `(state, now, url, "", &mut corr)`，并修 `increment` 冷启动断言 `cursor == 0`（非 42，HX-C008）。
4. **调用方**（`module.rs` 等 `ImWsContext::new` 真实构造点）：传入真 `auth_user_id`（host 经 ctx 注入）。
5. **`sync_session.rs`**：`PostFields` 加 `viewers` 字段（见 3.1.1）。
6. **依赖前置**：`channel_write/`（Phase B：`program_full`/`collect_present`/`collect_members`/`post_updates`）+ `acl/to_effect_s1.rs`（`channel_set_cols`/`upsert_channel_members`/`update_member_nickname`/`emit_*`）+ `acl/to_effect.rs` 增量（`upsert_channel_full`/`update_channel_partial`/`bump_channel_unread`/`emit_post_batch_updated`）必须先落地。

---

## 4. 分发流（WS 帧 → Effects）—— mermaid 编号

1. driver 收 WS 文本帧 → `Tick::Inbound(bytes)` 喂 `ImModule`。
2. `WsFrame::parse(bytes)` → 解出 `action`（顶层 `action`/`event`）+ `data`。
3. `registry` 用 `action` 在 `WS_HANDLERS` 构建的 `HashMap<&str, &dyn WsMessageHandler>` 做 O(1) 查表。
4. 命中 → `handler.handle(&mut ctx, &frame, &mut out)`；未命中 → `UnsupportedWsAction`（或 ignored 名单静默）。
5. handler 解析 payload（`frame.data_required()` + `extract_post_fields`/`parse_*`，边界零信任，缺字段→no-op）。
6a. **内容类**（5/6/7 post_update/posts_update/post_read）→ `gate.rs` → `Channel::ingest`（严格 +1 / drop / buffer+arm 1s gate）→ gate kind-aware push 落库 op + emit。
6b. **channel 写类**（3/8/9/10 increment/channel_created/member_update/update_channel）→ `channel_write::{program_full|collect_present|collect_members}` → `to_effect{,_s1}::{upsert_channel_full|update_channel_partial|upsert_channel_members}` push Persist。
6c. **定点列类**（11/12/13/9' nickname/schedule_*/channel_close）→ `to_effect_s1::{channel_set_cols|update_member_nickname}` push BatchUpdate/Upsert。
6d. **path3**（1 post）→ `channel_write::post_updates` → `to_effect::bump_channel_unread`（SQL `col=col+1` O(1)，仅 contiguous && !echo）。
7. 各 handler `out.push(emit_*)`（`Effect::Emit` 信封，前端总线透传）。
8. `out: EffectSink`（`Vec<Effect>`）整批回 driver 解释（**批处理过边界**，HX-C007）。

---

## 5. 逐 action 验收断言（来自 `tests/s1_ws_actions_test.rs`，576 行）

- **post_read**：(a) `post_read_overwrites_read_bits_and_emits_s1`：写 `message` 表，`patch[read_bits]==Text("1010")`（覆盖式），emit `im:post:read`；event_seq 须 == cursor+1（harness 种 0 → seq=1）。(b) `post_read_missing_post_id_no_message_write_s1`：缺 postId → 不写 message。
- **post_update**：`post_update_edits_content_only_and_emits_s1`：写 `message`，是 `BatchUpdate` 含 `message` 列且 **不含 `read_bits`**（edit_content_op 不擦已读位），emit `im:post:updated`。
- **posts_update**：(a) `posts_update_batch_revoke_and_edit_s1`：revoke 行 `patch[revoke]==Integer(1)`，编辑行 `patch[message]==Text("edit2")`，emit `im:post:batch-updated`（**不**发 `im:post:updated`），payload 含 `"posts"` 数组。(b) `posts_update_missing_posts_array_noop_s1`：缺 posts → effects 空。
- **channel_created**：(a) `channel_created_full_upsert_and_members_s1`：写 `channel` 表（全量 upsert），`channel_member` upsert row `user_id=="u-aaa"`（memberChange.join），emit `im:channel:created`。(b) `channel_created_missing_id_noop_s1`：缺 id → effects 空。
- **increment_channel（A1）**：`increment_channel_writes_channel_member_table_a1`：写 `channel`（path1）+ `channel_member`；四角色行 role：u-mem=MEMBER / u-adm=ADMIN / u-boss=BOSS / u-own=OWNER；emit `im:channel:increment`。
- **channel_close**：`channel_close_sets_delete_at_and_inactive_s1`：`patch[delete_at]==Integer(1700000000)` + `patch[is_active]==Integer(0)`，emit `im:channel:closed`。
- **channel_member_update**：(a) `channel_member_update_full_and_members_s1`：写 `channel`，`channel_member` row `role=="ADMIN"`（MANAGER→ADMIN normalize），emit `im:channel:member-updated`。(b) **`channel_member_update_cold_start_cursor_seed_zero_hxc008`**：真新 channel + `lastEventSeq=999` → 内存 `cursor_for==Some(0)`（**不是 999**），`increment_target_for==Some(999)`（解耦）。
- **nickname**：(a) `nickname_update_writes_member_nick_and_emits_s1`：`channel_member` upsert row `nick_name=="新昵称"`，`conflict_key==Some("channel_id,user_id")`（复合 PK），emit `im:channel:memberNickname`。(b) `nickname_update_missing_user_id_noop_s1`：缺 userId → 空。
- **schedule**：`schedule_created_sets_flag_s1`：`patch[has_schedule_post]==Integer(1)`，emit `im:channel:schedule-created`；`schedule_canceled_clears_flag_s1`：`==Integer(0)`，emit `im:channel:schedule-canceled`。

> 测试 harness：`Harness::new(&[channel_ids])` 预种 cursor=0；`h.step(inbound(json), now_ms)` → 断 `h.sink` 的 StorageOp + emit 名。内容类 action 须 `event_seq == cursor+1` 才立即 apply（否则进 gate buffer，不落库）。

---

## 6. 排序依赖（哪些 handler 必须在 Phase B / channel_write 之后落地）

**前置链（必须先于 Phase C 任何依赖 handler）**：
1. `sync_session.rs::PostFields` 加 `viewers` 字段 → `parser.rs` 才能编译。
2. `ws/context.rs` 加 `auth_user_id` → 所有用 `ctx.auth_user_id` 的 handler（post.rs path3 / increment / channel_created / channel_member_update）。
3. **Phase B = `channel_write/`**（`program_full`/`collect_present`/`collect_members`/`post_updates`）+ `acl/to_effect_s1.rs` + `to_effect.rs` 增量（`upsert_channel_full`/`update_channel_partial`/`upsert_channel_members`/`bump_channel_unread`/`emit_post_batch_updated`）。

**必须在 Phase B 之后的 handler**（依赖 channel_write / to_effect_s1，dev-zlc 当前**两者都没有**）：
- **强依赖 channel_write**：`channel_created.rs`、`channel_member_update.rs`、`update_channel.rs`、`increment_channel.rs`(改) 的 path1、`post.rs`(改) 的 path3。
- **依赖 to_effect_s1**：`channel_close.rs`、`post_schedule_created.rs`、`post_schedule_canceled.rs`、`update_channel_member_nickname.rs`（用 `channel_set_cols`/`update_member_nickname`/`emit_*`）。

**可独立先行（仅依赖 gate + parser，不碰 Phase B）**：`gate.rs`（最先）、`post_update.rs`、`posts_update.rs`、`post_read.rs`——这三个内容类 handler 只需 `gate.rs` + 既有 `channel.rs`（`edit_content_op`/`revoke_op`）+ parser。`gate.rs` 必须先于这三者。

**建议落地顺序**：
1. `sync_session.rs`(viewers) + `parser.rs` + `context.rs`(auth_user_id) + registry/调用方适配（前置基建）。
2. `gate.rs` → `post_update.rs` / `posts_update.rs` / `post_read.rs`（内容类，不阻塞 Phase B）。
3. **Phase B**（`channel_write/` + `to_effect_s1.rs` + `to_effect.rs` 增量）。
4. `channel_created.rs` / `channel_member_update.rs` / `update_channel.rs`（path1/path2，依赖 B）。
5. `channel_close.rs` / `post_schedule_*.rs` / `update_channel_member_nickname.rs`（定点列，依赖 to_effect_s1）。
6. 改 `post.rs`(path3) + `increment_channel.rs`(path1+成员) + `increment_channel_end.rs`(stuck.sort)（既有 handler 增量，依赖 B）。
7. 最后 `mod.rs` 把 10 个新文件全声明（distributed_slice 注册），跑 `s1_ws_actions_test.rs` 全绿。
