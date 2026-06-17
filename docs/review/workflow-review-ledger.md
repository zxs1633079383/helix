# Agent Workflow Review Ledger — 架构不偏移收敛台账

> **规则**：[`.claude/rules/agent-review-cadence.md`](../../.claude/rules/agent-review-cadence.md)
> **闸门脚本**：`scripts/review-gap.sh`（算 HEAD 距上次 review 的 commit 距离）
> **review workflow**：`Workflow({name:"architecture-drift-review"})`

## 不变量（强制收敛）

```
gap = git rev-list --count <last_reviewed_commit>..HEAD
gap ≥ 10  →  🔴 强制：在 commit 检查点（非中途任务）派发 Workflow({name:"architecture-drift-review"}) 收敛后，方可继续 feature 开发
review 通过 → 把 last_reviewed_commit 更新为本次 review 的 HEAD + 追历史行 → gap 归 0
```

目的：任何 agent / workflow 自主改动累计**不超过 10 个 commit** 就必须过一次架构 review（HX-C001..C008 不变量 + GOAL §1/§2 设计理念 + 全流程合理性：收发→解析→cursor→落库），防止「跑着跑着偏离架构」无人察觉。**只在 commit 检查点判定，绝不中途任务触发**（不打断实现）；命中可自动派 drift-review（无需问用户，token 高性价比）。

## 当前状态

```
last_reviewed_commit: e501952
last_reviewed_at: 2026-06-17
last_review_runId: wf_744e7f2e-3e2 (architecture-drift-review，五轮 HttpFire 背压根治 + 治理 medium 收口)
# 合并支线（main 侧 WS registry 重构线）最后一次独立 review = 00df4e0
# (agent_019ed4c3-0a53-7dd0-a7a9-bef44483cdad，WS registry merge 收口)
# 本次 dev-zlc ← main 合并（integ/merge-main-2026-06-17）尚未过 drift-review：
# 合并把两条改动线（dev: corr_map+InboundEvent 重构 / main: WS handler registry）汇成
# registry-as-dispatch + post/echo/HttpFire 能力并集，下一个 commit 检查点须派
# architecture-drift-review 收敛后再把 last_reviewed_commit 推进到合并 HEAD。
threshold: 10
```

> 说明：2026-06-16 规则生效，基线锚点 `8c69f25`。首轮核 `8c69f25..3c5324f`（🟡 2，已修 `cff383f`）。二轮 gap=10 核 `3c5324f..6b2fd52`（merge ADR-010），🟡 5：D2/D4/D5 即修、D1/D3 登记。三轮 gap=13 核 `6b2fd52..761bf50`（sync-v2 C1/C2/C3/B2 + eventbus/迁移文档），🟡 3：emit 信封即修(`43bf75d`)、type1 克隆/echo 真帧登记，ledger 推进到 `761bf50`。四轮收口推进到 `d052a19`（A/B 重构 + §4.8 治理）。**五轮 gap=11 核 `d052a19..5745931`**（HttpFire 背压根治 core/driver/im + 兄弟会话 §4.8/HX-C009 治理元系统），🟡 轻：8 条预判全证实**无一破 HX-C001..C009/GOAL §1/§2/三层/sans-IO/单调 cursor**——2 medium（drift-review 首跑被 SESSION:4 图例自污染·grep 未锚 §0 + §4.8 机制未 dogfood）即修 `ad6f0ec`/`e501952`，6 low（HttpFire 空桶 YAGNI→option-C 文档化 / recv 锁注释诚实化 / 7ecede5 tripwire 名实 / 治理元系统 scope creep）登记择期。ledger 推进到 `e501952`（含 medium 修复，非直接跳 5745931=避免「把空转闸门登记为已收敛」的作弊）。核心不变量 HX-C001/C005/三层/sans-IO 全未破。

## 待收敛 todo（drift-review 登记）

- [x] **[medium] HX-C005 热路径回归** ✅ 已修 commit `cff383f`（方案 A：PostFields 提取上移 parser，event_to_upsert_op 零再解析；B 列名 &'static str 受 core Row 契约所限退，注释如实）：`event_to_upsert_op`（channel.rs:296-353，6d83ad5 引入）WS 每事件 `serde_json::from_slice(raw)` 全量重解析 + ~14 次 String 堆分配；sync 路径 parse→reserialize(parser.rs:313)→parse 三趟。SQL 写仍真 O(1)（单 BatchUpsert ON CONFLICT，无 RMW），受 WS 帧率约束非 fan-out → medium。**修法**：A. 字段提取上移 parser，EventEnvelope 携 owned typed 视图（消除 reserialize）→ **与 B5（Post map→message 内容落库）同源，建议合并到 B5 轮**；B. 列名 `&'static str`；C. channel.rs:293 注释诚实化（SQL O(1) 但分配非零）；D. 承压实测 alloc 数。
- [x] **[low] 双 cursor schema 占位** ✅ 已修 commit `cff383f`（从 IM_SCHEMA 删 channel_event_cursor 死表，cursor 真源 = _helix_monotonic）。

### 二轮（merge 收口）drift——D1/D3 前置触发 todo

- [ ] **[medium·D1] FFI 句柄不透明类型不一致**（接 Swift/Kotlin FFI 冒烟前必修）：`helix-driver-ffi/src/api.rs` create/destroy/on_ws_frame/on_http_response 用 `*mut c_void`，但 `helix_create_with_network` 返 `*mut HelixHandle`——HX-C006 单一句柄类型契约。修法：整套统一 `*mut HelixHandle`（或统一 c_void + 单点 cast helper）+ #Safety 注释改 `helix_create_with_network`。修后 HX-C006 §3 补「一逻辑句柄=一 opaque type」+ recurrence 1→2。当前 on_ws_frame/on_http_response 是 TODO 桩从不解引用 → 无 UB，故 medium 非 high。
- [ ] **[medium·D3] native·FFI header registry 模型不对称**（接 WS reconnect 前必修）：native HTTP(http.rs)/WS(transport.rs) 各建独立 `HostHeaderRegistry`；FFI 共享单一（ADR-010 要求统一注入 HTTP+WS）。当前 host-cli 仅启动期一次灌头 → 维护性 footgun；WS reconnect 落地后陈旧快照脱节会变正确性 bug。修法：host-cli 改 `SharedHttpClient::with_registry`+`SharedWsClient::with_registry` 传同一 registry + `NativeHttp::from_shared`/`NativeTransport::from_shared`，身份头只灌一次（删 ws_headers 重复路径）。建议 native CLAUDE.md 登记「共享 registry」不变量。
- [x] **[medium·D2] NativeHttp::new() 活墓碑 + live bench caller** ✅ 已修（本轮）：删 `new()`/`with_timeout()`（误用变编译错）+ bench:119 改 `with_config(HostNetworkConfig::new(...))` + 删 tombstone 测试。
- [x] **[low·D4] ADR-008→010 文档引用漂移** ✅ 已修：02-driver-native/04-driver-ffi/ROADMAP 三处 `ADR-008`→`ADR-010`（保留 006/007）。
- [x] **[low·D5] decisions.md:62 transport 适配滞后注** ✅ 已修：更新为「transport.rs 已适配 SharedWsClient + 真 Go 22s 回归通过」。

### 三轮（sync-v2 C 收口）drift

- [x] **[medium·R3-emit] emit_post_received 信封不一致** ✅ 已修 commit `43bf75d`：msg_id 填 event_seq 数字+缺 event_seq → 改 `(channel_id, event_seq:u64, msg_id:&str)` 对齐同批 + cses post.rs:1252；3 调用点(sync+WS×2)同步。
- [ ] **[low·R3-clone] type1 sync 落库每事件 2× PostFields 克隆 + 无用 msg_id 派生**（HX-C005 常数因子，~16 alloc 可降 ~8）：to_effect.rs:482-487 `fields.clone()`+`EventEnvelope::new`（内部再派生 msg_id）；channel.rs event_to_upsert_op 从不读 ev.msg_id。**修法（可选优化，非强制）**：抽 `upsert_op_from_fields(channel_id,seq,&PostFields)` 借用映射，type1 复用，删中间 EventEnvelope::new + 无用派生。分支仍 O(1) 无 RMW，核心不变量未破。
- [ ] **[low·R3-echo] echo 识别强耦合 event_seq 存在**（pending-verification，gated D 阶段真帧）：parser.rs:281-289 event_seq 三级 fallback 尾 `?`，全缺→classify_echo None→posted 退 Ignored→reconcile 不触发。真源 cses post.rs:58 echo 仅凭 temporary_id 非空、与 event_seq 解耦。已诚实标「真帧待核 TODO」。**修法**：D 阶段 UC-1 真 Go 抓首个真 echo 帧核对 action/字段路径落 fixture；若真帧无 event_seq→fallback 降 `Option<Seq>`、echo 只凭 temporary_id+id+channel_id 三件、seq=None 跳 advance_cursor（与真源解耦对齐）。

### 四轮（A/B 重构）drift — dev-zlc 侧改动线

- [x] **[medium·R4-echo-gate] post echo reconcile 被 channel 注册门控** ✅ 已修 commit `e7858a9`（reconcile 提到 channel 门控外 + 回放测试 echo_reconciles_even_when_channel_unregistered + 架构设计 §4.5 解耦不变量注）：把 `reconcile_post_echo` 整个包在 `if let Some(ch)=channels.get_mut(&ev.channel_id)` 内，而 reconcile 只读 pending_sends/corr_map、从不读 channels → 门控多余且有害。冷启动 send-before-cursor-seed 乱序窗口下 pending 残留 → 15s on_timeout 误标 UnSend。**修法**：reconcile 提到 channel 查找之外（先无条件 reconcile，再 `if let Some(ch)` 做 ingest/cursor）。**合并后真源**：该解耦不变量已随 dev post 路径搬进 `ws/handlers/post.rs::reconcile_post_echo`（无条件先做，再 `if let Some(ch)` ingest），R4-echo-gate 修复存活。
- [x] **[low·R4-retain] reconcile_post_echo 用 corr_map.retain O(n) 清 OptimisticSend** ✅ 已修 commit `e7858a9`（send 回填 persist_corr → corr_map.remove O(1)，删 retain）。**合并后真源**：`ws/handlers/post.rs` + `ws/handlers/posted.rs` 两路径均用 `corr_map.remove(persist_corr)` O(1)。
- [x] **[low·R4-doc-rot] module.rs 陈注引用已删 corr_to_* 表** ✅ 已修 commit `e7858a9`：函数体已用单表 corr_map+match CorrelationContext。
- [x] **[low·R4-tombstone] parse_inbound_tick 零调用方活墓碑** ✅ 已修 commit `520568f`（删 parse_inbound_tick 保留 accepts_tick + 清失活 import + 更新 A-spec item8）。

### 四轮（WS handler registry T1-T3/T4-T7）drift — main 侧改动线

- [x] **[medium·R4-registry-mainpath] registry 尚非完整生产入站主路径** ✅ 已修 commit `00df4e0`（merge `codex-ws-handler-registry`）：`Tick::Inbound` 生产主路径改为 `WsFrame::parse` → `dispatch_ws`，注册 hello / increment / increment_end / posted / ignored / legacy handlers；unknown action 进入生产路径并返回 `UnsupportedWsAction`；invalid JSON 保持 no-op；旧 `FrameClass` / `classify_frame` / `parse_hello` / `classify_echo` 已删除。**合并后**：dev 的 `post` action 主路径以新增 `ws/handlers/post.rs` handler 形态接进 registry（action=="post"），post/echo/HttpFire 能力并集保住。
- [ ] **[low·R4-hotpath-parse] legacy WS 路径仍有一次额外 parse 常数因子**：top-level action 主路径为单次 `WsFrame::parse`；但 legacy no-action `{event,data}` 仍在 `legacy_event` handler 内 `parse_inbound(frame.raw())` 再解析一次。未破 O(1)，作为后续优化保留。**修法（可选）**：让 `WsFrame`/legacy handler 复用已解析 `root/data`，减少 raw 重解析。

## 历史

| 日期 | review 到 commit | 区间(from..to) | workflow runId | drift 判定 | 关键发现 / 收敛动作 |
|---|---|---|---|---|---|
| 2026-06-16 | 8c69f25 | — (基线) | — | 🟢 baseline | 规则生效锚点；无 review，仅设基线 |
| 2026-06-16 | 3c5324f | 8c69f25..3c5324f | wf_ee61a139-68b | 🟡 轻 drift | 2 条真 drift（8 报告去重）：[medium] event_to_upsert_op 热路径二/三次 serde + per-event alloc（违 HX-C005，6d83ad5 引入）；[low] channel_event_cursor 死表占位 vs 运行时 _helix_monotonic。均非阻塞 → 登记 todo（已于 cff383f 收敛）|
| 2026-06-16 | 6b2fd52 | 3c5324f..6b2fd52 (10 commits) | wf_703e8a49-178 | 🟡 轻 drift | merge main→dev-zlc 收口。5 条真 drift（11 报告去重）：D1 FFI 句柄类型不一致(medium)；D2 NativeHttp::new 活墓碑+live bench caller(medium)；D3 native·FFI registry 不对称(medium)；D4 ADR-008→010 三处文档引用漂移(low)；D5 decisions.md:62 transport 适配滞后注(low)。无机制破坏、core 零运行时仍成立。D2/D4/D5 即修，D1/D3 登记前置触发 todo |
| 2026-06-17 | 761bf50 | 6b2fd52..761bf50 (13 commits) | wf_71a48486-004 | 🟡 轻 drift | sync-v2 C1/C2/C3/B2 + eventbus/迁移文档收口。3 条真 drift（5 报告去重）：R3-emit emit_post_received 信封 msg_id=seq 数字+缺 event_seq(medium，已修 43bf75d)；R3-clone type1 落库双重 PostFields 克隆+无用派生(low，HX-C005 常数因子)；R3-echo echo 识别耦合 event_seq(low，pending-verification gated D 阶段真帧)。核心不变量 HX-C001/C005/三层/sans-IO 全未破 |
| 2026-06-17 | d052a19 | 761bf50..d052a19 (10 commits) | wf_f8be5840-1a0 | 🟡 轻 drift | corr_map 统一(B)+InboundEvent 合并(A) 重构 + refactor-spec-then-build workflow 留痕。6 报告去重为 4 条真 drift：R4-echo-gate post echo reconcile 被 channel 注册门控致冷启动乱序漏对账 15s 误标 UnSend(medium，登记择期修，非阻塞)；R4-retain corr_map.retain O(n) 清 OptimisticSend(low，HX-C005 常数因子非真违)；R4-doc-rot module.rs:143-149/939/969 陈注引用已删 corr_to_* 表(low)；R4-tombstone parse_inbound_tick 零调用方活墓碑(low)。核心不变量 HX-C001/C005/C008/三层单向/sans-IO/单调 cursor 全未破，GOAL §1/§2 未触 |
| 2026-06-17 | e501952 | d052a19..e501952 (11 commits) | wf_744e7f2e-3e2 | 🟡 轻 drift | 五轮 HttpFire 背压根治(core/driver/im) + §4.8/HX-C009 治理元系统。8 预判全证实无破 HX-C001..C009/GOAL §1/§2/三层/sans-IO/单调 cursor；2 medium 即修(ad6f0ec/e501952)、6 low 登记择期 |
| 2026-06-17 | 5f33446 | 761bf50..5f33446 (13 commits) | agent_019ed44f-6da4-7c83-90eb-179a05ab066a | 🟡 轻 drift | WS handler registry T1-T3 收口（main 改动线）。2 条真 drift：R4-registry-mainpath registry 当前只覆盖 hello+legacy，unknown-action 仍未进入 production 主路径(medium，按 Task 4-7 收敛)；R4-hotpath-parse legacy 临时多 parse/copy 常数因子(low，随 Task 7 单 parse 收敛)。HX-C001/C002/C004/C006/C007/C008 未破，core/runtime/FFI/sync 契约保持 |
| 2026-06-17 | 00df4e0 | 5f33446..00df4e0 (11 commits) | agent_019ed4c3-0a53-7dd0-a7a9-bef44483cdad | 🟡 轻 drift | WS handler registry merge 收口（main 改动线）。R4-registry-mainpath 已收敛：生产入站统一 `WsFrame::parse` → registry handler，unknown action 返回错误，invalid JSON 保持 no-op；`FrameClass`/`classify_frame` 无残留。HX-C001/C002/C004/C005/C006/C007/C008 未破；仅 AGENTS/CLAUDE EOF 空行镜像漂移(low，收尾即修) |
| 2026-06-17 | (待 review) | 994b9ee(merge-base)..合并HEAD | integ/merge-main-2026-06-17 | ⏳ 待 drift-review | **dev-zlc ← main 合并收口**：两条改动线汇合——dev 的 corr_map 统一 + InboundEvent 一趟解析(A/B 重构) + post/echo + core HttpFire；main 的 WS handler registry 策略注册表。和解方案 = registry 做 dispatch 主干，嫁接 dev 的统一 corr_map+CorrelationContext，dev post action 主路径以新增 `ws/handlers/post.rs` handler 接进 registry（含 R4-echo-gate 解耦 + reconcile 15s BLOCKING 修复 + corr_map 防泄漏）。判定待下个 commit 检查点派 architecture-drift-review，通过后把 last_reviewed_commit 推进到合并 HEAD |
