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
last_reviewed_commit: 6b2fd52
last_reviewed_at: 2026-06-16
last_review_runId: wf_703e8a49-178 (architecture-drift-review，二轮 merge 收口)
threshold: 10
```

> 说明：2026-06-16 规则生效，基线锚点 `8c69f25`。首轮核 `8c69f25..3c5324f`（🟡 2 条，已修 `cff383f`）。二轮 gap=10 触发，核 `3c5324f..6b2fd52`（merge main→dev-zlc + ADR-010 共享 client + 真 Go 硬修），🟡 5 条：D2/D4/D5 已即修，D1/D3 登记前置触发 todo，ledger 推进到 `6b2fd52`。

## 待收敛 todo（drift-review 登记）

- [x] **[medium] HX-C005 热路径回归** ✅ 已修 commit `cff383f`（方案 A：PostFields 提取上移 parser，event_to_upsert_op 零再解析；B 列名 &'static str 受 core Row 契约所限退，注释如实）：`event_to_upsert_op`（channel.rs:296-353，6d83ad5 引入）WS 每事件 `serde_json::from_slice(raw)` 全量重解析 + ~14 次 String 堆分配；sync 路径 parse→reserialize(parser.rs:313)→parse 三趟。SQL 写仍真 O(1)（单 BatchUpsert ON CONFLICT，无 RMW），受 WS 帧率约束非 fan-out → medium。**修法**：A. 字段提取上移 parser，EventEnvelope 携 owned typed 视图（消除 reserialize）→ **与 B5（Post map→message 内容落库）同源，建议合并到 B5 轮**；B. 列名 `&'static str`；C. channel.rs:293 注释诚实化（SQL O(1) 但分配非零）；D. 承压实测 alloc 数。
- [x] **[low] 双 cursor schema 占位** ✅ 已修 commit `cff383f`（从 IM_SCHEMA 删 channel_event_cursor 死表，cursor 真源 = _helix_monotonic）。

### 二轮（merge 收口）drift——D1/D3 前置触发 todo

- [ ] **[medium·D1] FFI 句柄不透明类型不一致**（接 Swift/Kotlin FFI 冒烟前必修）：`helix-driver-ffi/src/api.rs` create/destroy/on_ws_frame/on_http_response 用 `*mut c_void`，但 `helix_create_with_network` 返 `*mut HelixHandle`——HX-C006 单一句柄类型契约。修法：整套统一 `*mut HelixHandle`（或统一 c_void + 单点 cast helper）+ #Safety 注释改 `helix_create_with_network`。修后 HX-C006 §3 补「一逻辑句柄=一 opaque type」+ recurrence 1→2。当前 on_ws_frame/on_http_response 是 TODO 桩从不解引用 → 无 UB，故 medium 非 high。
- [ ] **[medium·D3] native·FFI header registry 模型不对称**（接 WS reconnect 前必修）：native HTTP(http.rs)/WS(transport.rs) 各建独立 `HostHeaderRegistry`；FFI 共享单一（ADR-010 要求统一注入 HTTP+WS）。当前 host-cli 仅启动期一次灌头 → 维护性 footgun；WS reconnect 落地后陈旧快照脱节会变正确性 bug。修法：host-cli 改 `SharedHttpClient::with_registry`+`SharedWsClient::with_registry` 传同一 registry + `NativeHttp::from_shared`/`NativeTransport::from_shared`，身份头只灌一次（删 ws_headers 重复路径）。建议 native CLAUDE.md 登记「共享 registry」不变量。
- [x] **[medium·D2] NativeHttp::new() 活墓碑 + live bench caller** ✅ 已修（本轮）：删 `new()`/`with_timeout()`（误用变编译错）+ bench:119 改 `with_config(HostNetworkConfig::new(...))` + 删 tombstone 测试。
- [x] **[low·D4] ADR-008→010 文档引用漂移** ✅ 已修：02-driver-native/04-driver-ffi/ROADMAP 三处 `ADR-008`→`ADR-010`（保留 006/007）。
- [x] **[low·D5] decisions.md:62 transport 适配滞后注** ✅ 已修：更新为「transport.rs 已适配 SharedWsClient + 真 Go 22s 回归通过」。

## 历史

| 日期 | review 到 commit | 区间(from..to) | workflow runId | drift 判定 | 关键发现 / 收敛动作 |
|---|---|---|---|---|---|
| 2026-06-16 | 8c69f25 | — (基线) | — | 🟢 baseline | 规则生效锚点；无 review，仅设基线 |
| 2026-06-16 | 3c5324f | 8c69f25..3c5324f | wf_ee61a139-68b | 🟡 轻 drift | 2 条真 drift（8 报告去重）：[medium] event_to_upsert_op 热路径二/三次 serde + per-event alloc（违 HX-C005，6d83ad5 引入）；[low] channel_event_cursor 死表占位 vs 运行时 _helix_monotonic。均非阻塞 → 登记 todo（已于 cff383f 收敛）|
| 2026-06-16 | 6b2fd52 | 3c5324f..6b2fd52 (10 commits) | wf_703e8a49-178 | 🟡 轻 drift | merge main→dev-zlc 收口。5 条真 drift（11 报告去重）：D1 FFI 句柄类型不一致(medium)；D2 NativeHttp::new 活墓碑+live bench caller(medium)；D3 native·FFI registry 不对称(medium)；D4 ADR-008→010 三处文档引用漂移(low)；D5 decisions.md:62 transport 适配滞后注(low)。无机制破坏、core 零运行时仍成立。D2/D4/D5 即修，D1/D3 登记前置触发 todo |
