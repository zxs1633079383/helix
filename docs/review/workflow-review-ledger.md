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
last_reviewed_commit: 0514a4d
last_reviewed_at: 2026-06-18
last_review_runId: wf_b809b3b3-ab6 (十一轮 architecture-drift-review：F5 跨端 parity + F4.5 demo 开局 · 区间 e3540cf..0514a4d · gap=11 触发)
# 十一轮 = sync-v2 re-port 开局前置 gate（gap 11 命中自动派）。🟡 轻 / 0🔴。
# 区间 e3540cf..0514a4d（11 commit：F5 跨端 parity 自动化 + F4.5 Flutter demo 开局 EXPECT/spec）。
# 3 finding 全 low 非阻塞 backlog：F5 egress parity 测试旁路真泵 per-step flush ×2（test-fidelity）
# + Cargo.toml 日语注释 ×1（code-style）。无 HX-C/GOAL 违反、byte-parity 结论可信、无新建卡。
# 注：0514a4d 后 dev-zlc 另有 2 docs commit（4db4592/c3f1797 = F4.5 demo 指针+SESSION，纯文档无 src）gap=2 绿。
# 另·F4.5 会话收口期因陈旧 gap 锚点(e3540cf·并发十一轮推进发生在本轮 drift-review 运行的 492s 内)并发误触发**重叠** drift-review wf_fd406543-c76
#   (区间 e3540cf..c3f1797 ⊇ 十一轮 e3540cf..0514a4d)，结论一致 🟡 轻·无 HX-C001..C011 / GOAL §1·§2 / 三层·sans-IO·单调cursor 架构违反。
#   独有 finding F3 = a80cc49 C++ smoke harness DONE 缺配对 🎯EXPECT(§4.8 audit 后门，与 finalize DONE 同根≥2 逼近建卡阈值)
#   → 候选 HX-C012『实质任务(含 test/CI/harness ≥50 行或新源文件)须开局 EXPECT』回放闸门 **HOLD**(历史 trap 均代码级约束·性质不同源·挡不住这次·待下次再过)。
#   last_reviewed 不推进(闸门已由十一轮满足 gap=2 绿·避免与活跃并发会话争写主字段)；F1/F2 与十一轮 F5-egress-parity finding 同源已登记。
last_review_prev_runId: wf_3f017dd5-500 (十轮：收口+设计思想+HX-C011 · 区间 cb8b6d5..6d90461)
# 十轮 = 收口代办+三端设计思想沉淀+新建 HX-C011 review（cb8b6d5..6d90461，gap=10 触发）。🟡 轻 / 0🔴。
# 3 finding 全证实全收敛(commit e3540cf)：F1 core effect.rs DropOldest 文档残留→DropNewest；
# F2 stress_storage_core 2 false-green 补 break→fail 断言 + HX-C011 §4 gate 文件级→函数级；
# F3 finalize DONE 缺配对 EXPECT→补事后 EXPECT。HX-C001..C011/GOAL 全未破。
# 函数级 gate 升级后抓出 helix-im stress_inorder_100k 亦 false-green=backlog(留 im/feat 线修)。
# 九轮 = storage 读写并发压测（feat/storage-rw-concurrency 2 commit · merge cb8b6d5）。**test-only 无架构变更**。
# verify-diff 2 lens(test-validity + HX-C005/C006/迁移)判 0 blocking；首版 2 false-green(P1-C tautology / P3 无 assert)
# 被 test-validity lens 抓出 → 修 + 证伪证明(破坏→FAIL→还原→PASS)。P0 path3 dev-zlc 零 RMW 合规；feat path3 flag 合并待查。
# c229531..cb8b6d5 间 b2dcf26/b92012c 等为 docs(ledger/设计 mmd/ROADMAP/SESSION)，无代码 drift。
# 八轮(c229531)详见下历史行。
# 八轮 = driver-ffi C++ 直连 + M3-F4 pump 端到端（branch design/ffi-cpp-direct 11 commit，merge c229531）。
# 七轮(0156be8)曾把「FFI engine_loop 未接线」对抗驳回为「ROADMAP M3-F4 计划内」——本轮即把 M3-F4 落地：
# pump 核下沉 helix-driver-host（run_engine_loop 泛型壳），native(1119→168)+ffi 共用薄壳 parity by construction。
# branch drift-review 判 light / 0 blocking：3 条 ⚡ DRIFT-PRED(scope 扩/design 臆造/query stub)全「有据偏离」、
# 过不了回放闸门均不建卡（与 HX-C009/runtime-agnostic §6 重叠）。4 minor 收敛 commit 2967350 后 merge。
# 注：0156be8..8997a72 为 docs/chore(crate 索引+design mermaid+Cargo.lock)，无代码 drift，gate.sh 全闸门覆盖。
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
| 2026-06-17 | 1419690 | e501952..1419690 (gap=29) | wf_e9045934-1e9 | 🟢 GREEN 无 drift | **六轮 = dev-zlc ← main 合并收口 review**（681a529 merge + 1419690 结构 rule）。两条改动线汇合——dev 的 corr_map 统一 + InboundEvent 一趟解析(A/B 重构) + post/echo + core HttpFire；main 的 WS handler registry。和解 = registry 做 dispatch 主干 + dev post/echo/HttpFire 能力并集（新建 `ws/handlers/post.rs` + 3 文件 corr_to_sync→corr_map+CorrelationContext::SyncPull）。多维对抗 review（合并正确性/架构 drift/结构 rule，12 agents）：8 条确认 finding **全 low/nit 无阻断**——MERGE-1 非echo posted 漂移 error!日志(low)、MERGE-2 legacy 双解析回退(low)、DRIFT-1/RULE doc-rot+死链(low/nit)。**HX-C001..C009 + helix-im 五不变量 + GOAL §1/§2 全 HOLD**，cargo check 四 crate + test 165/165 + core 零运行时 grep 已绿。8 条登记择期，非 land 前置 |
| 2026-06-18 | 0156be8 | 1419690..0156be8 | wf_ba3cc60b-dae + wf_69cf287f-163 | 🟢 GREEN 无 drift | **七轮 = core/driver 基座最优性 + 北极星符合性 双深度审查**（8 专家 agent + 对抗证伪）。基座：core-ports `solid`(6 trait dyn-safe + MaybeSend cfg 别名 + Module::handle 拿不到 port 句柄=sans-IO 类型保证) / HTTP-头 `solid` / WS-action `solid` / Effect `solid-minor-gaps`。北极星：5 命题全 conforms（一份 core 编多端✓ 复杂度无损迁移 FfiStorage 薄壳✓ 三层单向零业务✓ sans-IO/确定性✓ 扩展性✓）。**2 个 latent bug 修+回归测试(2d50405)**：EFFECT-3 多模块 start() corr 覆盖(P0,威胁第二模块插得进) + NS-1 resync HashMap 迭代序非确定性(high,威胁可回放)。打磨(0156be8)：EFFECT-5/2 执行契约文档 + HTTP-4 parking_lot 消静默丢头。**7 疑似 drift 驳回**(FFI engine_loop=ROADMAP M3-F4 计划内非偏移)；**8 优化建议对抗驳回**(防过度工程)。CORE-1/WS-1 微 nit + 候选 HX-C010(确定性约束,过回放闸门后建卡)择期 |
| 2026-06-18 | c229531 | 8997a72..design/ffi-cpp-direct (11 commits · merge c229531) | wf_8b8c9a1f-cc5 | 🟡 轻 drift | **八轮 = driver-ffi C++ 直连 + M3-F4 pump 端到端**（实现优先 + 编译器/clippy 闸门 + 验真 diff，非 design-doc 轮——前两轮 design-doc 验证连栽臆造 core 类型 9 blocking，pivot 后一轮收敛）。下沉整套 pump 核到 helix-driver-host(run_engine_loop 泛型壳/dispatch/BoundedSpawner/run_http_envelope/TimerRegistry/consume_effects)，native 1119→168 薄壳 + ffi 薄壳，**parity by construction**（七轮驳回的 M3-F4 计划内项本轮落地）。3 lens(HX 不变量/GOAL+全流程自洽/DRIFT-PRED 对抗证伪)判 light/0 blocking：3 条 ⚡ DRIFT-PRED(scope 扩 host/design 臆造/query stub)全「有据偏离」、过回放闸门均不建卡(重叠 HX-C009+runtime-agnostic §6)。4 minor(文档滞后/FfiTransport·FfiStorage 死包装 rm/build.rs helix.hpp 脏化/composition-root 注)收敛 2967350。**HX-C001(core 零运行时)/C005/C006/C007 + GOAL §1/§2 + 三层单向 全 HOLD**；native 零回归(18 lib+stress 3+4)+ffi 12(含 e2e command→Emit→批回调)+host 17 全绿 + gate.sh 7 闸门过。留 TODO：helix_query stub(name→Spec 属 IM 域,待 IM 暴露通用 query 路由) |
| 2026-06-18 | cb8b6d5 | b92012c..feat/storage-rw-concurrency (2 commits · merge cb8b6d5) | wf_45201637-38e | 🟡 轻（test-only） | **九轮 = storage 读写并发压测**（下沉后共享核覆盖 + sync-v2 写负载，用户点出）。test-only 无 src 行为变更。verify-diff 2 lens(test-validity / HX-C005-C006-迁移)判 0 blocking。**首版 2 false-green 被抓**：P1-C cursor 单调 tautology(顺序写恒 100·删 MAX guard 照过) + P3 bench 无 assert + block_on 单 op 不跑并发 → 修 + **证伪证明**(P1-C 去 guard→FAIL/还原→PASS·P1-A 去背压→FAIL)。P0：dev-zlc channel.unread_count 本轮零写路径=零 RMW(HX-C005 合规)·cursor=monotonic_upsert 原子 ON CONFLICT；feat/sync-v2 path3 flag 合并待查未跨改。host 34+native stress 8 零回归·gate.sh 过·core 零运行时。候选教训「压测必带证伪证明」与 2026-06-15 stress-verify REJECT 同根=2 次，未达建卡阈值待回放闸门 |
| 2026-06-18 | e3540cf | cb8b6d5..6d90461 (gap=10·收敛 e3540cf) | wf_3f017dd5-500 | 🟡 轻 drift | **十轮 = 收口代办+三端设计思想沉淀+新建 HX-C011**（DropOldest→DropNewest 改名 + 压测下沉 host + 三端 00-设计思想.md + HX-C011 建卡 + P1-A flaky hotfix）。3 finding 全证实全收敛(e3540cf)：F1[low] core effect.rs:241/taxonomy.mmd DropOldest 文档残留(改名黑名单遗留)→DropNewest；F2[low] stress_storage_core 2 测无 assert false-green + HX-C011 §4 grep -L 文件级对混合文件逃检 → 补 break→fail 断言(scan len + monotonic 回退) + gate 升函数级；F3[medium] finalize DONE 缺配对 EXPECT(§4.8 tripwire 失效)→补事后 EXPECT。**HX-C001..C011 + GOAL §1/§2 + 三层/sans-IO/单调 cursor 全未破，代码合规**。回放闸门：F2(b) gate 修=N/A·F3(b) DONE须配EXPECT 结构约束=HOLD(单样本无同根史)。函数级 gate 抓出 helix-im stress_inorder_100k=backlog。「**HX-C011 建卡即首次执行就抓到同 diff 违反**」=机制即时生效验证 |
| 2026-06-18 | 0514a4d | e3540cf..0514a4d (11 commits·gap=11 触发) | wf_b809b3b3-ab6 | 🟡 轻 drift | **十一轮 = sync-v2 re-port 开局前置 gate**（gap 11 命中自动派）。区间 = F5 跨端 parity 自动化 + F4.5 Flutter demo 开局 EXPECT/spec。3 finding 全 low 非阻塞 backlog：F5 egress parity 测试旁路真泵 per-step flush ×2（test-fidelity·crates/helix-driver-ffi/tests/f5_egress_parity.rs）+ Cargo.toml 日语注释 ×1（code-style）。**无 HX-C/GOAL 违反、byte-parity 结论可信、无新建卡**。0514a4d 后 dev-zlc 另有 2 docs commit(4db4592/c3f1797=F4.5 demo 指针·纯文档)gap=2 绿。下游 = sync-v2 全 parity re-port（branch feat/sync-v2-report-on-adr007） |
