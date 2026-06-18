# Merge Spec — main → dev-zlc 调和合并（WS registry × sync-v2 corr_map）

> 文档类型：合并过程合成 spec（事后记录 + 决策依据 + 验证结论 + 回滚预案）
> 日期：2026-06-17 · 分支：`integ/merge-main-2026-06-17` · merge-base：`994b9ee`
> Ours/HEAD = dev-zlc 侧（`21c13ad`）· Theirs/MERGE_HEAD = main 侧（`559db4e`）
> 状态：5 冲突文件 + 6 隐藏编译债文件已全部解完，工作树零冲突标记，四 crate 编译通过，全量测试绿。**未 commit、未 abort。**

---

## ① 背景：两分支并行重构撞车

main 与 dev-zlc 在同一时间窗对 `crates/helix-im` 的 **WS 帧 dispatch 主干**各自做了一次大重构，且两条线触碰同一组核心文件（`module.rs` / `parser.rs` / `state.rs` / 测试），形成「双重构撞车」式合并：

- **main 侧改动 = WS handler registry 重构**
  把原本集中在 `module.rs` 的 action 分发，替换为**策略注册表**：新增 `crates/helix-im/src/ws/{registry,frame,context,mod}.rs` + `ws/handlers/{hello,ignored,increment_channel,increment_channel_end,legacy_event,posted,mod}.rs`。用 `linkme::distributed_slice(WS_HANDLERS)` 静态注册 + `OnceLock` 懒建 `HashMap` + `build_registry` 校验，`dispatch_ws` 按 `frame.action()` 查表路由，未知 action → `UnsupportedWsAction`（生产可观测）。该架构已被 drift-review 认可，扩展性优于集中 match。

- **dev-zlc 侧改动 = sync-v2 C1 主路径 + 统一 corr_map + HttpFire**
  ① `parser.rs` 的 `InboundEvent` 一趟解析（`parse_hello`/`classify_inbound`/`classify_echo`/`parse_post_frame` + `Post`/`Echo` 变体）= sync-v2 C1 真 Go `post` action 在线广播 + echo `temporary_id` 对账；
  ② `helix-core` 的 `HttpFire` Effect 变体（`effect.rs` + `engine.rs` 通配臂不注册 corr）+ `helix-driver-native` `BoundedSpawner`（`DropOldest` 背压可丢 HTTP）；
  ③ `sync_session.rs`；
  ④ `state.rs` 统一 `corr_map` + `CorrelationContext` 状态模型（`corr_to_sync`/`corr_to_channel`/`corr_to_send`/`continuation_pending`/`scan_corr` 五者删除合一）。

两条线都把「WS dispatch」当作改造对象，但一条改的是**骨架形态**（集中 match → registry），另一条改的是**状态模型 + 能力**（多张 corr 表 → 单 corr_map，新增 post/echo/HttpFire）。它们正交但耦合在同一批文件——naive 整取任一侧都会丢另一侧能力或直接编不过（详见 ⑦）。

---

## ② 影响范围面（文件级）

### 2.1 五个冲突文件（需人工解冲突）

| # | 文件 | 冲突块 | 性质 |
|---|---|---|---|
| 1 | `crates/helix-im/src/module.rs` | 5 块（含 152/105 行大块）| dispatch 接缝：dev 集中 match vs main 委托 registry |
| 2 | `crates/helix-im/src/parser.rs` | 1 块（252 行）| dev 整块 `InboundEvent`/`classify_*`/`parse_post_frame`；main 侧此处为空 |
| 3 | `crates/helix-im/tests/increment_parse_test.rs` | 6 块 | 测试断言两侧分叉（classify_inbound 内省 vs registry 路径）|
| 4 | `crates/helix-im/tests/real_go_frames_test.rs` | 1 块（4 行）| doc 注释措辞 |
| 5 | `docs/review/workflow-review-ledger.md` | 3 块 | drift 台账历史行（非代码语义）|

### 2.2 main-only 自动带入文件（16 项，无冲突标记但部分含隐藏编译债）

WS registry 全族无冲突自动进工作树：
`ws/context.rs` · `ws/frame.rs` · `ws/mod.rs` · `ws/registry.rs` · `ws/handlers/{hello,ignored,increment_channel,increment_channel_end,legacy_event,posted,mod}.rs` ·
`docs/superpowers/plans/2026-06-17-ws-handler-registry.md` · `docs/superpowers/specs/2026-06-17-ws-handler-registry-design.md` ·
另含 `Cargo.toml`（linkme 依赖）/ `error.rs`（`UnsupportedWsAction`）/ `lib.rs`（mod ws）等。

> ⚠️ **隐藏编译债（plan 点名最易漏）**：`state.rs` 已无冲突自动并到 dev 侧（`corr_map` 赢、`corr_to_sync` 删），但以下三个 **无冲突自动带入** 的 main 文件仍引用已删符号，不改则 `cargo check` 直接失败：
> - `ws/handlers/increment_channel_end.rs:47` — `corr_to_sync.insert` → 必须改 `corr_map.insert(SyncPull{channel_id})`
> - `ws/registry.rs`（test:676）— 同改
> - `tests/post_action_inbound_test.rs:417` — 引用已删 `classify_inbound`/`InboundEvent` 内省 → 移除

### 2.3 dev-only 自动带入文件（13+ 项）

`helix-core/src/{effect.rs,engine.rs}`（HttpFire）· `helix-driver-native/src/engine_loop.rs`（BoundedSpawner）· `helix-im/src/{state.rs,sync_session.rs,acl/from_tick.rs}` · `tests/{golden_replay_test.rs,post_action_inbound_test.rs,echo_reconcile_test.rs}` · fixtures `real_go_frames/*` · `docs/harness/HX-C009-*.md` 等。这些 dev-only 文件均无冲突，关键是 merge 后须确认 HttpFire/BoundedSpawner 未被回退（已验证）。

---

## ③ 业务流影响（WS 帧 → parse → gate → registry/post echo → effect → persist）

合并后的稳定主链路：

```
WS 帧 (Tick::Inbound)
  └─ WsFrame::parse(raw)            坏 JSON → Ok(()) 静默丢（边界零信任，不升 CoreError）
       └─ dispatch_ws_frame → dispatch_ws by frame.action()  （registry HashMap 单次查表）
            ├─ action=="post"   → ws/handlers/post.rs        ← dev C1 主路径（新增）
            │     ├─ parse_post_frame: channel_id 硬 gate / event_seq 三级(data.event_seq 优先) / 不硬校验 26 字符 id / extract_post_fields(data)
            │     ├─ reconcile_post_echo: temporary_id 命中在途 PendingSend → ps.reconcile(send_status='sent'+id=server_id+CancelTimer 撤 15s timer)
            │     │     · 无条件先 reconcile（R4-echo-gate 解耦，不被 channel 注册门控）
            │     │     · pending_sends.remove + corr_map.remove(persist_corr) 防泄漏
            │     │     · 不在 reconcile 推 cursor（交 ch.ingest，HX-C008 单调避免双推）
            │     └─ if let Some(ch) → ch.ingest(EventEnvelope)  ← echo/新消息不在落库层分叉，靠 ON CONFLICT(temporary_id) 自然对账（行为真源 router.rs:40 / message_service.rs:64）
            ├─ action=="posted" → ws/handlers/posted.rs        ← main legacy echo 兼容路径
            │     · parse_echo + 补 corr_map.remove(persist_corr)（防泄漏；PendingSend status=Sent 保留对齐 registry test 契约）
            ├─ 无 action 键      → legacy_event.rs (LEGACY_EVENT_ACTION) → parse_inbound → ch.ingest
            ├─ increment_channel / increment_channel_end → 对应 handler（corr_map+SyncPull）
            └─ 未知 action       → UnsupportedWsAction（生产可观测，取代 dev 静默 Ignored）
  └─ gate 三阶段 peek → 落库 → commit；cursor 单调不回退；too_long 清内存不清表
  └─ handle_port_reply: corr_map + CorrelationContext 四路由（ScanCursors/ChannelPersist/OptimisticSend/SyncPull）
```

三条落内容路径 `post`/`posted`/`legacy_event` 的 action 键互斥不重叠，HashMap 单次查表保证**每帧恰好一个 handler**，无双 dispatch / 无死代码 / 无绕过。

---

## ④ 调和决策与目标架构

**目标架构（一句话）**：**registry 做 dispatch 主干（保留 main 的 `ws/{registry,frame,context}.rs` + `ws/handlers/*` 策略注册表），嫁接到 dev 的统一 `corr_map` + `CorrelationContext` 状态模型，并把 dev 全部能力（post 在线广播 handler、reconcile_post_echo 含 15s BLOCKING 修复 + 清理、HttpFire）以「新增/增强 handler」形态接进 registry。**

三条决策理由：

1. **必须嫁接，不能整取**：`state.rs` 已无冲突自动并到 dev 侧（`corr_map` 赢，`corr_to_sync` 已删），main 的 `increment_channel_end.rs`/`registry.rs` 仍引用已删的 `corr_to_sync` → 「整取 main」根本编不过。
2. **骨架留 registry**：registry 是 drift-review 已认可、更可扩展的 dispatch 骨架；dev 的 `InboundEvent` 集中 match 与之等价但扩展性弱，二选一留 registry。
3. **能力并集而非文件择一**：dev 的 post/echo/HttpFire 是 sync-v2 C1 主路径 + BLOCKING 修复，绝不能丢，故走「registry 缺一块 → 新建 `post.rs` handler + 增强 `posted.rs` handler」的能力并集。

---

## ⑤ 逐冲突解决记录

### 5.1 `parser.rs`（risk: high）— 取 main，搬走 parse_post_frame

整块取 main：删除 HEAD 的 252 行（`InboundEvent` enum + `classify_inbound`/`parse_hello`/`classify_echo`/`parse_post_frame`）。dispatch 改走 registry by action，不再需要集中 classify。

- **保命动作**：`extract_post_fields` 从 private 改 `pub(crate)`，供新 `post.rs` 复用（HX-C005 零再解析）。
- **逐字不动**：`parse_inbound_value` 四级 `event_seq` fallback（边界等价铁律，测试断言依赖）；`parse_sync_response` 的 `nextSeq` 锚 + `needs_continuation=len>=500` + 分桶排序（HX-C008）。
- `parse_post_frame` 逻辑逐字搬到新 `post.rs` 作 private fn（channel_id 硬 gate / event_seq 三级 data.event_seq 优先 / 不硬校验 26 字符 id）；`classify_echo` 语义并入 `posted.rs`（main 已有 `parse_echo` 等价实现）。

### 5.2 `module.rs`（risk: high）— 骨架取 main，能力搬进 handler

5 块逐一：① doc 注释取 main（registry 措辞）。② `emit_proactive_resync_for` 取 main 委托体，删 HEAD 的 `handle_increment`/`handle_increment_end`/`heal_incomplete_backfill_once` 三个死方法（逻辑已在 ws handlers）。③ `handle_echo` → 取 main 的 `dispatch_ws_frame`；冲突块外、已无调用方的 `reconcile_post_echo` 整段删除（搬进 `post.rs`）。④ `Tick::Inbound` 臂取 main（`WsFrame::parse` → `dispatch_ws_frame`）。`handle_hello`（`#[allow(dead_code)]`）保留作历史/测试副本。

- **must_preserve（已保留）**：`handle_port_reply` 的 `corr_map` + `CorrelationContext` 四路由完整；`maybe_continue_sync`/`handle_sync_reply`/`handle_send_message`（依赖 corr_map）；`Disconnected` 臂 `corr_map.retain(SyncPull)` 清理。

### 5.3 `increment_parse_test.rs`（risk: medium）— 实质全取 main

dev 侧 `classify_inbound`/`InboundEvent` 已删，无对应 API，故 6 块全取 main（改 `Tick::Inbound` → registry 路径断言 `cursor_for`/`increment_fetched`/`emit`）。`event_seq` 三级 fallback 用例走 `parse_inbound`（两侧相同）保留。

### 5.4 `real_go_frames_test.rs`（risk: low）— doc 措辞取 main

1 块（4 行）：HEAD `InboundEvent::Ignored` → main `registry ignored handler`。取 main，因 dispatch 主干是 registry。

### 5.5 `workflow-review-ledger.md`（risk: low）— 三块 union

非代码语义纯台账：① 当前状态块保留两侧历史，`last_reviewed_commit` 保守仍取 dev 侧 `e501952`（见下方决策点）。② 四轮 drift 段 union（dev `R4-echo-gate/retain/doc-rot/tombstone` + main `R4-registry-mainpath/hotpath-parse`，标注两条改动线）。③ 历史表 union 全部既往行 + 追一行本次 merge 收口行（区间 `994b9ee..合并HEAD`，judgement ⏳ 待 drift-review）。

### 5.6 额外 6 个隐藏编译债文件（铁律 1 之外的必改）

执行铁律「只改这 5 个冲突文件」在实操中无法满足「编译过 + 保能力」，按 plan `capabilities_at_risk` 预警的「无冲突文件却编不过的隐藏 merge 债」覆盖铁律 1，额外改：
1. 新建 `ws/handlers/post.rs`（否则 dev post 主路径无 handler 落 `UnsupportedWsAction`，C1 主路径丢）；
2. `ws/handlers/mod.rs`（注册 post）；
3. `ws/handlers/posted.rs`（补 `corr_map.remove` 防泄漏）；
4. `ws/handlers/increment_channel_end.rs`（`corr_to_sync` → `corr_map+SyncPull`）；
5. `ws/registry.rs`（test 同改）；
6. `tests/post_action_inbound_test.rs`（移除已删 `classify_inbound` 内省断言，改同构造器正反夹逼）。

---

## ⑥ 验证结论（逐项）

| 项 | 命令 | 结论 |
|---|---|---|
| **双端 check** | `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi --message-format=short` | **PASS** — `Finished dev ... in 2.51s`，零 error / 零 warning，四 crate 全过 |
| **Go 帧冒烟** | `cargo test -p helix-im` | **PASS** — 23 个测试 binary 全通过，0 失败。`real_go_frames_test` 5/5（hello 握手 emit established / 运维帧零 effect / 非法 JSON noop / unknown action → UnsupportedWsAction / 显式 ignored noop）；`golden_replay_test` 5/5；`increment_parse_test` 12/12；`post_action_inbound_test` 7/7；`gate_test` 10/10；`send_test` 6/6；`stress_test` 3/3；`proptest_gate` 5/5；lib unit 34/34 |
| **单测** | `cargo test -p helix-core` | **PASS** — 3 个 binary 全过（1+4+1 passed，1 ignored），0 失败 |
| **零运行时 grep** | `cargo tree -p helix-core \| grep -iE 'tokio\|sqlx\|reqwest\|web-sys\|tauri'` | **PASS（空）** — grep EXIT=1 零匹配，HX-C001 成立。linkme 仅在 helix-im，不污染 core |

dev 能力逐条可达（测试佐证）：post action 在线广播（`post_frame_at_cursor_plus_one_persists_and_advances`）· echo temporary_id 对账（`echo_post_frame_reconciles_via_temporary_id_upsert`）· 15s BLOCKING 修复（`post_echo_then_15s_timer_does_not_mark_unsend`）· R4-echo-gate 解耦（`echo_reconciles_even_when_channel_unregistered`）· HttpFire 在 core 保留。

**全部验证项无 FAIL。**

---

## ⑦ drift 判定 + 能力丢失核查

**drift verdict：🟢 GREEN（无丢失能力）。**

不变量逐条 hold：

| 不变量 | 状态 | 要点 |
|---|---|---|
| HX-C001 core 零运行时 | hold | cargo tree grep 空；linkme/HttpFire 全在 core 内无 IO |
| HX-C005 热路径 O(1)/&'static | hold | `extract_post_fields` 一趟提取，无 `from_slice(raw)` 二次解析；reconcile 走 `corr_map.remove` O(1)（非全表 retain 扫描）；落库单 upsert ON CONFLICT |
| HX-C008 cursor 锚 nextSeq | hold | `parse_sync_response` next_seq = nextSeq 兜底 max(event.seq)，`needs_continuation=len>=500`；post/posted reconcile 不推 cursor |
| helix-im-1 只依赖 helix-core | hold | registry/handlers 仅 use helix_core + linkme（非 runtime）|
| helix-im-2 handle 纯函数 | hold | 所有 ws handler 副作用只经 `out.push(Effect)`，无 await/IO/spawn |
| helix-im-3 行为真源对齐 | hold | post.rs 锚定 router.rs:40 + message_service.rs:64 ON CONFLICT(temporary_id) |
| helix-im-4 边界零 unwrap | hold | 缺字段 → None no-op / Err；坏 JSON → Ok(()) 不升 CoreError |
| helix-im-5 gate 三阶段 | hold | peek→落库→commit；too_long 清内存不清表（无 DELETE）；cursor 单调 try_advance |

**能力丢失核查**：plan `capabilities_at_risk` 预警的全部陷阱均已规避——
- ✅ 没整取 main（否则丢 post/echo/HttpFire）；没整取 dev（否则丢 registry 骨架）；
- ✅ 隐藏编译债（`corr_to_sync` 引用）已全修，无「无冲突文件却编不过」；
- ✅ posted.rs 升级补了 `corr_map.remove`（无 corr_map/PendingSend 泄漏）；
- ✅ post handler 的 reconcile 放在 channel 查找**之外**（无条件先做），未重新引入 R4-echo-gate 冷启动乱序漏对账 bug；
- ✅ ledger 两侧 drift 历史 union 保留，审计链未断；
- ✅ HttpFire 在 engine `register_effects_to_maps` 通配臂归属未被回退。

**flow_coherent：自洽**——`frame.action()` 单 HashMap 查表保证每帧恰好一个 handler，post/posted/legacy_event 动作键互斥；post(C1 主路径) 与 posted(legacy 兼容) 的「保留 PendingSend vs 全清理」差异是有意设计而非 bug；unknown→UnsupportedWsAction 是有意的生产可观测升级（drift-review 已认可）。

---

## ⑧ 残留风险与回滚

### 残留风险（均为非阻塞）

1. **ledger `last_reviewed_commit` 取值是设计决策点**：方案原拟「取本次 merge HEAD」，但 merge 尚未 commit（HEAD 未知）且合并本身**未过 drift-review**——直接把 last_reviewed 跳到未审 commit = 闸门作弊（`agent-review-cadence.md §5` 禁止）。故**保守保留 dev 侧 `e501952` 为 last_reviewed** + 注明 main 支线 `00df4e0` + 历史表追一行 ⏳ 待 drift-review 的 merge 收口行，judgement 留给 commit 后的检查点派 `architecture-drift-review` 再推进。这是合规取舍，非阻塞。
2. **铁律 1 被覆盖**：实操改了 5 冲突文件之外的 6 个文件（已在 ⑤.6 逐一交代），按「保能力 + 编译过」优先级覆盖「只改 5 文件」，已留痕。

### 回滚预案（本合并在 integ 分支，代价低）

- 本合并完全隔离在 `integ/merge-main-2026-06-17`，未触碰 `dev-zlc` / `main`。
- 一键回滚：`git merge --abort`（合并尚在进行中，工作树未 commit）即可丢弃全部解冲突结果，回到合并前。
- 或 commit 后弃用：`git checkout dev-zlc && git branch -D integ/merge-main-2026-06-17`。
- 无任何对 main / dev-zlc 的破坏性写入，回滚零风险。

---

## ⑨ 落地建议

**整体可平稳 commit**——四 crate check 全过、helix-im/helix-core 全量测试 0 失败、零运行时 grep 空、drift 判定 🟢 green、能力丢失核查全部规避。无 FAIL / 无 red。

建议落地顺序：

1. **可直接 commit** 本合并（结构化 commit body：影响范围列 5 冲突 + 6 隐藏债文件 / 改动影响面 / 功能改进 = registry × corr_map 调和 / Harness 更新 = HX-C001/C005/C008 hold / 验证 = 上表四项 PASS）。
2. **commit 后第一件事**：在该 commit 检查点**自动派** `Workflow({name:"architecture-drift-review"})`，区间含 merge-base `994b9ee..合并HEAD`，把 ledger 历史表那行 ⏳ 的 judgement 收口、并把 `last_reviewed_commit` 合规推进到已审 commit（当前保守停在 `e501952` 是为避免闸门作弊）。
3. **无需主对话中途介入**——两处残留风险均为已记录的合规取舍，不阻塞 commit。

---

*（spec 由合并现场事实 + 调和方案 + 解冲突结果 + 验证结果 + drift 对抗五份材料合成；文件路径均为仓库内绝对/相对锚点，便于 review 定位）*
