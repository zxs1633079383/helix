---
id: HX-C008
title: sync/notify pull 契约——cursor 锚服务端 nextSeq（非 max(event.seq)）+ 续拉看 len≥500（无显式 flag）
status: active
severity: blocker
scope: helix
created: 2026-06-16
source: 2026-06-16 用户 challenge「这是 cses-client 和 go 的语义吗」→ 核 cses-client handlers/post.rs + cses-im-server internal/app/sync.go 实证
recurrence_count: 2
trigger_paths:
  - "crates/helix-im/src/ws/parser/core/sync.rs"
  - "crates/helix-im/src/sync/flow.rs"
  - "crates/helix-im/src/sync/session.rs"
trigger_keywords:
  - "parse_sync_response"
  - "SyncEntry"
  - "nextSeq"
  - "needs_continuation"
  - "SYNC_EVENTS_PER_ENTRY"
  - "sync/notify"
related:
  - HX-C005
graduates_to:
  - AGENTS.md   # 「事件流单调 cursor」节
inline_target:
---

# HX-C008 — sync/notify pull 契约护栏

## §1 触发场景（Trigger）

改 `parse_sync_response` / `handle_sync_reply` / `SyncResponse`；新增/改任何消费 `POST /api/cses/channel/sync/notify` 响应的代码；问「sync 怎么续拉 / cursor 推到哪 / 真 wire 长啥样」。grep：`parse_sync_response`、`SyncEntry`、`nextSeq`、`SYNC_EVENTS_PER_ENTRY`。

## §2 背景（why）

2026-06-16 修 B3（139 个 `sync response missing 'type'`：increment_channel_end 后的 sync 补漏路径拿回真 Go wire `{status,data:{entries:[SyncEntry]}}`，旧 parser 按废弃抽象 `{type:...}` 解）。用户 challenge「这是 cses-client 和 go 的语义吗」→ 强制核两个**行为真源**（GOAL §4.5），定下三条不可凭脑补的契约：

- **真 wire 形态**（cses-im-server `internal/app/sync.go` + cses-client `domain/sync.rs §7.1`）：
  `{status:"SUCCESS", data:{entries:[SyncEntry], serverTime}}`；
  `SyncEntry={channelId, kind, events:[ChannelEvent], nextSeq?, resetTo?, messages?}`；
  `ChannelEvent={eventSeq:int64, eventType:1/2/3/6, channelId, msgId?, actorId, createAt(★少个 d), payload?}`。
- **阈值**（Go 常量）：`SyncEventsPerEntry=500`、`SyncGapTooLongThreshold=5000`。
- **kind 决策树**（Go `syncOne`）：`gap≤0→no_change(nextSeq=maxSeq)` / `0<gap≤5000→events(GetSince limit=500)` / `gap>5000→too_long(resetTo=maxSeq)` / `snapshot 保留未启用`。

两条**最易脑补错**的点（本卡核心）：

1. **续拉无显式 flag**：真 wire `events` kind **没有** `needs_continuation` 字段。Go「饱和 slice」：gap>500 时返 500 条且 `nextSeq<maxSeq`，客户端**凭 len≥500 续拉**。cses-client `handlers/post.rs` 原话：「仅 Events 分页（**events.len() >= 500**）才续拉；Snapshot 不分页」。helix 当年抽象 wire 自造了 `needs_continuation` 字段 = 错位。
2. **cursor 锚权威 nextSeq，不锚 max(event.seq)**：Go `next = events[len-1].EventSeq`（非空时），客户端 cursor 跳到 nextSeq。当前契约下 `nextSeq == max(event.seq)`（恒等），但**显式锚 nextSeq 防契约漂移**——若尾部 phantom seq 不进 events 数组（§1.6 phantom：不可见但 cursor 仍推进），用 max(event.seq) 会让 cursor 落后 → 下次 sync from 仍是旧水位 → 服务端又算大 gap → **稀疏 sync 风暴**（cses-client 2026-06-03 实测根因，`state.rs` 注释「cursor 应直接推到 next_seq，而非 flush_contiguous 严格 +1 卡第一个空洞」）。
3. **type7 可以与可见前缀同批**：Go 按 `fromSeq` 返回当前 viewer 可见的整个事件段，不承诺 closed 必须单独成批。首次发现一个已关闭 channel 时，合法响应可以是 `[type1@1, type7@2] + messages + nextSeq=2`。客户端必须验证 type7 唯一且位于末尾，再把可见前缀、terminal tombstone 与 cursor 放入同一个 `PersistAtomic`；不能因 `event_count>1` 或 `messages` 非空拒绝后立即请求同一 cursor。

## §3 Required / Forbidden

```rust
// ✅ 续拉：满批派生（真 wire 无显式 flag），阈值 = Go SyncEventsPerEntry
pub const SYNC_EVENTS_PER_ENTRY: usize = 500;
let needs_continuation = events.len() >= SYNC_EVENTS_PER_ENTRY;

// ✅ cursor 锚服务端权威 nextSeq；缺省回退 max(event.seq)，再退 0
let max_event_seq = events.iter().map(|e| e.seq.0).max().unwrap_or(0);
let next_seq = Seq(entry["nextSeq"].as_u64().unwrap_or(max_event_seq));
// 消费侧 commit_target 取 max(next_seq, 实际末条)：异常响应 next_seq<末条时不丢已落库事件
let commit_target = next_seq.max(max_event_seq);

// ✅ closed 可带普通事件前缀；整批一次落库，回执前不推进内存、不发 Frame
assert!(terminal_is_unique_and_last);
assert_eq!(next_seq, terminal_seq);
PersistAtomic(prefix_message_ops + terminal_tombstone_and_cursor);

// ✅ ChannelEvent 游标字段是 eventSeq（int64），非 seq；类型是 eventType，非 event_type
let seq = ev["eventSeq"].as_u64().or_else(|| ev["event_seq"].as_u64())...;
```

```rust
// ❌ 自造 needs_continuation 字段当真 wire（旧抽象 wire 的错位，真 Go 无此字段）
let needs_continuation = v["needs_continuation"].as_bool();

// ❌ cursor = max(event.seq)：当前恒等但契约漂移会落后 → 稀疏 sync 风暴
let commit_target = events.iter().map(|e| e.seq).max();   // 缺了 nextSeq 锚定

// ❌ 按抽象 {type:...} 解；读 ev["seq"]/ev["event_type"]（真 wire 是 eventSeq/eventType）
let resp_type = v["type"].as_str()...;   // → 139 个 "missing 'type'"

// ❌ 对确定性拒绝响应用未变化 cursor 立即重拉；服务端会返回完全相同的批次并形成自激风暴
reject();
emit_proactive_resync_for(channel_id);
```

## §4 Verification

- grep gate：`grep -n 'needs_continuation.*as_bool\|v\["type"\]' crates/helix-im/src/parser.rs` 期望 **0 行**（不得回退抽象 wire）。
- grep：`grep -n 'SYNC_EVENTS_PER_ENTRY\|next_seq.max' crates/helix-im/src/{parser,module}.rs` 期望命中（续拉 + 锚定都在）。
- 单测：`cargo test -p helix-im --test gate_test sync_cursor_anchors_to_next_seq_not_max_event`（证 nextSeq=50 / max(event.seq)=10 时 cursor 跳 50）；`--test e3_continuation_test`（满批续拉 → no_change 收尾）。
- terminal 回归：`cargo test -p helix-im --test t095_terminal_event_rejection_test`；其中 mixed case 必须证明 `[type1@1,type7@2]` 只产生一个原子批次、回执后 cursor/terminal=2，且 HTTP 回包处理步不再产生新的 sync Http。
- 行为真源核对：`cses-im-server/internal/app/sync.go`（syncOne/syncOneEvents）+ `cses-client/src-tauri/src/features/im/{domain/sync.rs,handlers/post.rs}`。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-16 | B3 修复（本会话） | 139 个 `sync response missing 'type'` | parser 按废弃抽象 `{type:...}` 解，真 wire 是 `{status,data:{entries:[SyncEntry]}}` |
| 2026-07-22 | Native G16 重启恢复 | 16 个已关闭 channel 在约 118 秒内重复 WARN 6370 次，Go `sync/notify` 持续 200 | Helix 误认 type7 必须单事件且无 messages；拒绝合法 `[type1,type7]` 后用 cursor=0 立即重拉同一确定性响应 |

## §6 关联

- 上游：`docs/过会方案/helix-sync-v2-设计.md`（§1.4 sync/notify wire、§1.5 ChannelEvent、§1.6 phantom）；AGENTS.md §4.5 行为真源。
- 全局 rule：`AGENTS.md`「事件流单调 cursor」节（append-only event log + 单调 cursor + TooLong 兜底）——本卡是其 helix 落地。
- 兄弟 harness：HX-C005（热路径零分配——sync 批量落库走单 INSERT/UPSERT，禁 RMW）。
- 行为真源：cses-im-server `internal/app/sync.go` / cses-client `domain/sync.rs` + `handlers/post.rs`。

## §7 历史与演进

- drafting → active：2026-06-16 第一次命中（B3 修复 + 用户 challenge 触发实证核对）。
- 待办（未来若服务端契约变）：若 `messages` phantom map 投影（B5）落地，复核 nextSeq vs events 数组的关系是否仍恒等。
