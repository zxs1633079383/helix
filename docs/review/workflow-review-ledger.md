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
last_reviewed_commit: 5f33446
last_reviewed_at: 2026-06-17
last_review_runId: agent_019ed44f-6da4-7c83-90eb-179a05ab066a (architecture-drift-review，WS registry T1-T3 收口)
threshold: 10
```

> 说明：2026-06-16 规则生效，基线锚点 `8c69f25`。首轮核 `8c69f25..3c5324f`（🟡 2，已修 `cff383f`）。二轮 gap=10 核 `3c5324f..6b2fd52`（merge ADR-010），🟡 5：D2/D4/D5 即修、D1/D3 登记。三轮 gap=13 核 `6b2fd52..761bf50`（sync-v2 C1/C2/C3/B2 + eventbus/迁移文档），🟡 3：emit 信封即修(`43bf75d`)、type1 克隆/echo 真帧登记，ledger 推进到 `761bf50`。核心不变量 HX-C001/C005/三层/sans-IO 全未破。

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

### 四轮（WS handler registry T1-T3）drift

- [ ] **[medium·R4-registry-mainpath] registry 尚非完整生产入站主路径**：module.rs:955 仍先走 `classify_frame`，当前 production registry 只覆盖 `hello` + `__legacy_event__`；`increment_channel` / `increment_channel_end` / `posted` / ignored / unknown action 仍在旧 classifier 前置处理或静默忽略，registry unknown-action 测试只覆盖直接调用。**修法**：按既定 Task 4-7 把顶层 action 帧统一解析为 `WsFrame` 后进 registry，注册 increment/end/posted/ignored handlers，再收缩 `FrameClass`。完成前文档/交接必须明确“registry 当前仅覆盖 hello + legacy”。
- [ ] **[low·R4-hotpath-parse] legacy WS 路径临时多 parse/copy 常数因子**：分阶段接入后，legacy 路径为 `classify_frame` parse → `WsFrame::parse` parse+copy → `legacy_event` 再 `parse_inbound`；违反 HX-C005 零分配精神的常数因子，未破 O(1)。**修法**：随 Task 7 收敛到单 parse dispatch；`WsFrame` 尽量 cheap clone 原始 `Bytes`，handler 复用已解析 `root/data`，减少 raw 重解析。

## 历史

| 日期 | review 到 commit | 区间(from..to) | workflow runId | drift 判定 | 关键发现 / 收敛动作 |
|---|---|---|---|---|---|
| 2026-06-16 | 8c69f25 | — (基线) | — | 🟢 baseline | 规则生效锚点；无 review，仅设基线 |
| 2026-06-16 | 3c5324f | 8c69f25..3c5324f | wf_ee61a139-68b | 🟡 轻 drift | 2 条真 drift（8 报告去重）：[medium] event_to_upsert_op 热路径二/三次 serde + per-event alloc（违 HX-C005，6d83ad5 引入）；[low] channel_event_cursor 死表占位 vs 运行时 _helix_monotonic。均非阻塞 → 登记 todo（已于 cff383f 收敛）|
| 2026-06-16 | 6b2fd52 | 3c5324f..6b2fd52 (10 commits) | wf_703e8a49-178 | 🟡 轻 drift | merge main→dev-zlc 收口。5 条真 drift（11 报告去重）：D1 FFI 句柄类型不一致(medium)；D2 NativeHttp::new 活墓碑+live bench caller(medium)；D3 native·FFI registry 不对称(medium)；D4 ADR-008→010 三处文档引用漂移(low)；D5 decisions.md:62 transport 适配滞后注(low)。无机制破坏、core 零运行时仍成立。D2/D4/D5 即修，D1/D3 登记前置触发 todo |
| 2026-06-17 | 761bf50 | 6b2fd52..761bf50 (13 commits) | wf_71a48486-004 | 🟡 轻 drift | sync-v2 C1/C2/C3/B2 + eventbus/迁移文档收口。3 条真 drift（5 报告去重）：R3-emit emit_post_received 信封 msg_id=seq 数字+缺 event_seq(medium，已修 43bf75d)；R3-clone type1 落库双重 PostFields 克隆+无用派生(low，HX-C005 常数因子)；R3-echo echo 识别耦合 event_seq(low，pending-verification gated D 阶段真帧)。核心不变量 HX-C001/C005/三层/sans-IO 全未破 |
| 2026-06-17 | 5f33446 | 761bf50..5f33446 (13 commits) | agent_019ed44f-6da4-7c83-90eb-179a05ab066a | 🟡 轻 drift | WS handler registry T1-T3 收口。2 条真 drift：R4-registry-mainpath registry 当前只覆盖 hello+legacy，unknown-action 仍未进入 production 主路径(medium，按 Task 4-7 收敛)；R4-hotpath-parse legacy 临时多 parse/copy 常数因子(low，随 Task 7 单 parse 收敛)。HX-C001/C002/C004/C006/C007/C008 未破，core/runtime/FFI/sync 契约保持 |
