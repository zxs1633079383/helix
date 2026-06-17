# Spec — sync-v2 完成（Phase C/D 收尾至全验收）

> 状态：规划（2026-06-17）。基线 `c79dedd`（B3 真 wire + 678 持久化 + ADR-010 merge 已落）。
> 行为真源：cses-client `src-tauri/src/features/im/handlers/post.rs`（逐 file:line 已测绘，见本 spec 引用）。
> 验收总目标：**单元测试 + 真 Go :8065 直连冒烟 + 回归全绿**。
> 配套：`crates/helix-im/docs/sync-v2-能力与缺口状态.md`（缺口卡）· `docs/过会方案/helix-sync-v2-real-go-wire-设计.md`（wire 设计）。

---

## 0. 范围裁定（什么算「sync-v2 验收通过」）

| 纳入（正确性核心，必做） | 暂缓（非 sync-v2 核心，另立） |
|---|---|
| C1 echo→reconcile（发送对账 tmp_id→server_id）| C3 中**功能性事件**：schedule/member/nickname/todo/channel:created-closed 等（依赖未做的业务）|
| C2 phantom + 离线消息内容落库 + readBits（type6）+ edit/delete（type2/3）| D1 FFI 句柄类型统一（接 FFI 冒烟前才需，native 不阻）|
| C3-core：sync 驱动的 post:* mutation 事件（received/read/updated/deleted）+ `im:__bus__` 信封接线 | D3 native·FFI registry 对称化（接 WS reconnect 前）|
| B2 increment_end 冷启动自愈 | ping/退避重连（长连稳定性，单独）|
| D：golden fixtures（C1/C2/C3 代表帧）+ UC-1 端到端真 Go smoke | |

> 判据：**「连上 → 发出去对账回正 id → 离线 sync 补齐历史(含 phantom/已读/编辑/撤回) → 前端经 im:__bus__ 零改动消费」全链路真 Go 跑通** = sync-v2 验收。

---

## 1. C1 — echo→reconcile（5 缺口）

**现状**：`handle_echo`（module.rs:802-822，`#[allow(dead_code)]`）逻辑已备（tmp_id→TemporaryId、`ps.reconcile(server_id,...)`、cursor 独立推进），但**无人调用**——`classify_frame` 不认 echo 帧。
**真源**：cses-client 发送回流靠 `temporary_id` 匹配 → server `post.id` 覆写 message.id + send_status（post.rs handle_new_post）。

**实现**：
1. `parser.rs classify_frame`：加 `Some("echo")` 分支 → 新 `FrameClass::Echo { channel_id, seq, temporary_id, server_id }`（字段名对真帧核对，先抓一帧确认 action 名/字段）。
2. `sync_session.rs`：`FrameClass` 加 `Echo` 变体。
3. `module.rs` `Tick::Inbound` 分发：加 `Echo` 分支 → 调 `handle_echo`（去 `#[allow(dead_code)]`）。
4. `handle_echo`：除 `ps.reconcile` 外，补 **message 表覆写**——`UPDATE message SET id=server_id, send_status='sent' WHERE temporary_id=tmp_id`（StorageOp batch_update，对齐 678 列）。
5. 回归测试：`tests/echo_reconcile_test.rs`——发送→echo 帧→断言 PendingSend Sent + message.id 覆写 + cursor 推进。

**验收**：`cargo test -p helix-im echo` 绿 + 真 Go 冒烟见「发消息→收到自己消息 echo→id 由 tmp 变 server」。

---

## 2. C2 — phantom + 离线消息落库 + readBits（6 缺口）

**现状**：`parse_sync_response`（parser.rs:262-346）解析 4 kind，但 **`SyncEntry.messages`(Post map) 读了不落库**；`handle_sync_reply`（module.rs:512-604）只 `batch_upsert_events(events)`（只推 cursor，不落消息内容）。
**真源**：cses-client `batch_write_events`（post.rs:1043-1134）：`for event: if msg_id in messages map → upsert + apply read_bits(line 1087-1089)；else → phantom(仅 cursor，line 1271-1280)`。type=6 读 `messages[msg_id].readBits` **单调覆盖**（不 +1，直接赋值，line 1387-1436）。

**实现**：
1. `sync_session.rs`：`SyncResponse::Events` 加 `messages: HashMap<MsgId, PostFields>`（或 raw `serde_json::Value` map）。
2. `parser.rs parse_sync_response` events 分支：读 `entry["messages"]` → 解析每个 Post → `PostFields`（复用 `extract_post_fields`）+ read_bits。
3. `to_effect.rs batch_upsert_events`：签名加 `messages: &Map`——
   - type=1/2/3：`msg_id ∈ messages` → upsert message 行（含内容列）；`∉` → **phantom，不落 message 行**（cursor 照推）。
   - type=6：读 `messages[msg_id].read_bits` → message 表 `read_bits` 列**单调覆盖**。
4. `channel.rs event_to_upsert_op`：message 行加 `read_bits` 列（678 message 已有该列）。
5. `module.rs handle_sync_reply`：第 553 行改 `batch_upsert_events(&events, &messages_map)`。
6. type=2(edit)/3(revoke)：update message 行对应列（message/props 更新 / revoke=1）。

**铁律**：cursor 推进**独立于消息是否可见**（phantom 不卡 cursor，HX-C008 §1.6）；read_bits 单调覆盖非累加。

**验收**：`cargo test -p helix-im phantom read_bits` 绿 + 回放 fixture（sync 响应含 messages map + phantom + type6）+ 真 Go 离线>0 sync 后 message 表有内容行 + read_bits 正确。

---

## 3. C3-core — sync 驱动事件 + im:__bus__ 信封（核心子集）

**现状**：helix 发 7 个 emit（to_effect.rs:145-276：post:received/sending、sync:too_long、connection:established、channels:loaded、channel:increment/update）；`event_sink.rs` 只到 tokio broadcast，**未接 app.emit("im:__bus__")**。
**真源**：现网 24 事件总集；信封 `im:__bus__` 为 Tauri event name，payload 是事件 JSON。

**实现（只补 sync 驱动的 mutation 事件 + 总线接线，功能性事件暂缓）**：
1. `to_effect.rs`：补 sync/WS 驱动的 mutation 事件 emit——`im:post:read`(type6)、`im:post:updated`(type2)、`im:post:deleted`(type3)、`im:channel:update-by-post`(unread+N)。
2. `module.rs`：dispatch 在 type2/3/6 落库后 emit 对应事件。
3. `event_sink.rs` / driver-native：broadcast → `app.emit("im:__bus__", payload)` 接线（host-cli 侧用 NativeEventSink subscribe 消费打印验证；真 Tauri app.emit 留接缝，host-cli 无窗口故以 DomainEvent 接收计数验证）。
4. 信封格式：`im:__bus__` 为 event name，payload = 事件 JSON bytes（对齐现网）。

> 注：host-cli 无 Tauri 窗口，C3 验收以「DomainEvent 经 EventSink broadcast 收到 + 类型/数量正确」为准；真 app.emit 信封在 M2 Tauri 接缝验。

**验收**：`cargo test -p helix-im` emit 测试覆盖新事件 + 真 Go 冒烟 DomainEvent 计数含 post:read/updated/deleted。

---

## 4. B2 — increment_end 冷启动自愈

**真源**：cses-client `heal_incomplete_backfill_once()`（post.rs:666-699）——冷启动时若 increment 批次不完整，自愈补拉。
**实现**：`module.rs` increment_end(global) 处理时，检测 increment_fetched 与已落 cursor 的缺口 → 对未追平 channel 补发 sync。
**验收**：`cargo test -p helix-im increment_end` + 冷启动剧本测试。

---

## 5. D — golden fixtures + UC-1 端到端真 Go smoke

1. **golden fixtures**：`tests/fixtures/real_go_frames/` 现仅 `connect_init.jsonl`。用 `HELIX_DUMP_FRAMES=1` 抓真 Go 帧 → 补 echo / sync-with-messages / type2-3-6 序列 fixture（确定性 id/时间）。
2. **UC-1 端到端真 Go smoke**：host-cli 连 :8065——hello→increment→**发消息→echo 回正 id→离线 sync 补齐(phantom/read/edit/delete)→DomainEvent 流出**全链路；脚本扩 `scripts/dev-loop.sh`。
3. **回归**：`cargo test`（交付 6 crate）+ 真 Go 冒烟 + `scripts/gate.sh`。

**验收**：UC-1 全链路真 Go 跑通无回归 + 单测全绿 + 0 断连。

---

## 6. 执行编排（workflow + loop engine）

**依赖序**（文件高度重叠 module.rs/to_effect.rs/parser.rs → 顺序实现，不并行 worktree）：
```
C2(sync 落库/phantom/readBits) → C1(echo) → C3-core(事件+bus) → B2(自愈) → D(fixtures+UC-1 smoke) → 全回归
```
**每阶段闸门**：impl（写，白名单 helix-im + driver-native event_sink）→ 对抗 review（rust-tauri-skeleton-dev 核 cses 对齐 + rust-dev 正确性，READ-ONLY）→ blocking 回炉 → `cargo test -p helix-im` 绿。
**收尾**：真 Go :8065 直连冒烟（dev-loop.sh）+ 全交付 crate test + gate.sh。Go 未起则探测告警停。
**真源铁律**：每个行为对 cses-client post.rs 对应 file:line；HX-C008 cursor/续拉护栏不破；helix-core 零运行时不破。

---

## 7. 验收清单（全绿才算 sync-v2 通过）

- [ ] C1 echo：`cargo test -p helix-im echo` + 真 Go 发消息回正 id
- [ ] C2 phantom/readBits/落库：单测 + 回放 fixture + 真 Go 离线 sync 有内容行
- [ ] C3 事件+bus：emit 测试覆盖新事件 + DomainEvent 计数
- [ ] B2 自愈：冷启动剧本测试
- [ ] D fixtures + UC-1：真 Go 全链路 smoke 0 断连
- [ ] 回归：交付 6 crate `cargo test` 全绿 + `scripts/gate.sh` + core 零运行时
