# Helix 细粒度 Roadmap · TodoList · 验收 Checklist

> 2026-06-12 · 基于：技术过会方案 §6（M0-M4）+ 评审 D 系调整（+3 周）+ 实际进度对账。
> 2026-06-15 更新：按 ADR-006/ADR-010，网页端/wasm 暂停维护；产品化路线只保留 PC Tauri + Flutter 手机端；HTTP/WS client 与 Storage 一样走 shared Rust host driver，前端只负责命令、事件与渲染。
> 2026-06-16 更新：shared HTTP/WS client 代码边界已阶段落地到 `helix-driver-host`；native/ffi Rust 侧复用同一套 client 与 header registry。真 Go smoke、WS reconnect/backoff 与 proxy/TLS/cert knobs 仍按后续联调项推进。
> 2026-06-18 更新：**M2 ~90% 完**（native 真 Go 联调 + sync-v2 全链路 + shared host I/O + WS registry + 背压根治 + 基座最优性/北极星双 review 收口全落 main）；**M2 唯一剩余 = Tauri 四接缝接入（D1-3 接缝①②）**。**M3：F1 部分 / F2·F3 ✅，本次启动 F4（FFI engine 接线，拆解见 §一 M3）**。
> 当前位置：**按交付物 M2 收尾 + M3-F4 启动**（M1 全部 + M2 主体 + M3 F2/F3 已完）。

---

## 一、细粒度 Roadmap（天级）

### M2 收官 · 第 1 周——真连 Go 服务端（L2 联调）

> ⚠️ **2026-06-12 四专家 panel-review 重排**：原 D2-3 smoke 排在 B4/B5 之前 = 顺序倒置；且 UC-1 happy-path 跨的 3 块代码当时**死/缺/脆**（echo 死代码、parser 无 event_seq、`im:connection:established` 全仓 0 命中）。决策：**先补 4 块前置缺口再跑真 Go smoke**（用户拍板）。

- [x] **D1-2 最小 native 宿主**（`crates/helix-host-cli`）+ **UC-1 前置 4 块**——① hello 握手时序（`Tick::Connected`→发 hello→hello-ack→`im:connection:established`）✅ · ② B4 event_seq 三级 fallback + hello-ack 解析 ✅ · ③ B5 phantom ✅ · ④ echo→reconcile 路由（`handle_echo` 激活）✅。〔验收〕握手日志 + `im:connection:established` 可见 + 4 块各配 replay 测试 ✅
- [x] **D3-4 UC-1 端到端真 Go smoke**：发消息→0 等待上屏→echo 对账→查库 temporaryId 已换正式 id ✅（2026-06-17 真消息端到端验过）；golden fixtures 入库 ✅（`tests/fixtures/real_go_frames/`）。〔残〕im_trace→Tick 剧本完整工具链仍 P1
- [~] **D5** ~~E10~~（已落地 `d58dff1`）✅ + B2 increment_end 冷启动自愈 ✅（`20334d2`）+ 多模块并发写 benchmark：criterion 全链路覆盖 ✅；**多模块并发写实测仍开放**（单写者池在多业务模块下零实测，待第二模块）

### M2 收官 · 第 2 周——Tauri 换引擎（四接缝 + shared host I/O）

- [ ] **D1-3 接缝①②**：10 个生产命令内部切 Tick / 读旁路；`NativeEventSink` → `app.emit('im:__bus__')`；灰度开关。〔验收〕cses-client 前端零改动跑通收发。**未做**（M2 收官唯一剩余阻塞）
- [~] **D3-4 接缝③④**：同库同表灰度共存 ❌ + B7 store:\*/imDataSource SQL 语义对齐 ❌ + **B3 readBits 单调覆盖 ✅**（sync-v2 C2 落地）。〔验收〕开关切换 + 秒级回退演练
- [x] **D5 `helix-driver-host` shared HTTP/WS client 边界落地** ✅：`HostNetworkConfig` + shared header registry + native/ffi 薄壳复用，header/token 不进 IM。〔残〕真 Go smoke ✅、WS reconnect/backoff、proxy/TLS/cert knobs、旧 callback fallback 完整接线

### AI 工程化补课（0.5 周，可与上面并行——见 §四差距）

CI 双端 matrix（native + FFI）→ clippy deny-list → per-crate CLAUDE.md → cargo public-api gate（优先级从左到右）。wasm CI 不再作为必跑项，除非未来 ADR 重新激活网页端。

- [~] **F1 Flutter FFI API 最小面**：`create/destroy/set_header/remove_header/on_ws_frame/on_http_response` ✅；**缺 `start`/`helix_command`(send_message)/`helix_query`(读路径)** ❌
- [x] **F2 FfiStorage 复用 host** ✅：`StorageOp` 语义与 native 单一真源对齐，shared conformance 测试覆盖
- [x] **F3 shared HTTP/WS client + header API** ✅：FfiTransport(Shared+Callback) + FfiHttpRequester + `helix_set_header`/`helix_remove_header`；Dart 只注入 url/headers（demo 接线在 F4）
- [ ] **F4 engine 接线 + 事件回 Dart Stream**（**当前启动，拆解见下** ↓）
- [ ] **F5 跨端一致性验收**：同一 Tick 剧本 native vs FFI 输出 byte-level 一致 + schema/migration conformance + shared client 行为验收

#### ▶ F4 实装拆解（FFI 第二端 engine 接线 · 2026-06-18 启动）

> **现状**（`crates/helix-driver-ffi` 545 行）：opaque handle(create/destroy)✅ · FfiTransport/Http/Storage 薄壳✅ · set/remove_header✅；**但 FFI 无 engine_loop**——`helix_on_ws_frame`(api.rs:160)、`helix_on_http_response`(api.rs:187) 只拷贝 buffer 后 `// TODO` 不喂 engine；`on_domain_event` 回调表已定义但未接；无 command 入口。
> **核心缺口 = 没有「驱动 ExecutionShell 的事件泵」**（native 有 `engine_loop.rs` 1119 行，FFI 无对应）。

| 子任务 | 交付 | 验收 | 风险/依赖 |
|---|---|---|---|
| **F4.1 engine 泵归属决策（设计先行）** | ADR：native `engine_loop` 的 ExecutionShell 泵 + BoundedSpawner 抽进 `helix-driver-host` 共享（native/ffi 只配 runtime/transport 壳），还是 FFI 自写瘦泵。**推荐抽取共享**（ADR-007 单一真源，避免两套泵漂移）。定义 `EnginePump` 接缝（Tick 入队 / Effect 解释 / reply 回灌） | ADR 落 `decisions.md`；接缝签名定稿；native 若重构则回归全绿（零行为变更） | 🔴 最高：native 泵与 WS 连接生命周期耦合，抽取须解耦「连接管理」与「engine 泵」 |
| **F4.2 FFI engine runtime + Tick 入队** | `helix_create_with_network` 内建 tokio runtime + spawn engine 泵 task + `ExecutionShell.register(ImModule)` + `start()`；`helix_on_ws_frame`→拷贝→`Tick::Inbound`→channel 投 engine（接 api.rs:160 TODO）；`helix_on_http_response`→`PortReply{corr=req_id}`→投 engine（接 api.rs:187）；新增 `helix_command(handle, cmd_bytes)`→`Tick::Command`（send_message 入口） | FFI 收 WS 帧→engine step→FfiStorage 落库（→host）；mock callback 单测验证 | 依赖 F4.1；线程模型（current_thread runtime in Dart isolate） |
| **F4.3 事件回 Dart Stream** | `FfiEventSink impl EventSink`：`emit(DomainEvent)`→`callbacks.on_domain_event(ctx, bytes)`（接已定义回调表）；`Effect::Emit` 解释→FfiEventSink→Dart `ReceivePort/NativePort`→Stream | engine emit 的 DomainEvent 经回调到达 C 侧；跨 isolate 线程安全 | engine 泵在 tokio task，回调跨到 Dart isolate 须 NativePort |
| **F4.4 panic=abort + spawn_blocking 终核（MAJ-4）** | 确认 `panic="abort"` 在最终 cdylib（根 Cargo.toml✅）；核 FFI 路径同步阻塞经 host `spawn_blocking`（ADR-005 已澄清 storage 端 ×7 继承）；无新增裸同步阻塞 tokio worker | `cargo check -p helix-driver-ffi` + clippy + 边界测试（null/invalid-utf8/单 owner 已有） | HX-C006 铁律 |
| **F4.5 Flutter demo 冒烟 + 跨端一致（= F5 起点）** | cbindgen 生成 `helix.h`；最小 Flutter app `dart:ffi` 绑定 open/start/send/on_event；冒烟：Flutter 发消息→真 Go→echo 对账→事件回 Dart Stream 上屏 | Flutter 冒烟绿 + 同一 Tick 剧本 native vs FFI byte-level 一致（验收 §三 M3 项） | 依赖 F4.1-4.3 全绿 |

**关键路径**：F4.1（泵归属决策）是枢纽 + 最高风险，先出 ADR 定接缝再动手；F4.2/4.3 是主体接线；F4.4 是合规核；F4.5 兑现 F5 验收。**headless 守卫**：全程 core/helix-im 零改，只在 `helix-driver-ffi`（+ 可能 `helix-driver-host` 抽取泵）动；同一份 core 跑 ffi 零改。

### M4 —— 归档的 wasm/web（暂停维护）

`crates/helix-driver-wasm` 只作为历史实验保留（设计稿 `03-driver-wasm.md` 已随 docs 缩减删除，wasm 归档见 ADR-006）。当前不做网页端、不补 IndexedDB/OPFS、不把 wasm 纳入交付验收。未来若重启网页端，先新增 ADR 重新定义 storage/cache 策略和 CI。

---

## 二、TodoList（活账，按优先级）

> **2026-06-16 进展**：B1/B2 主体 + **B3 sync 真 wire**（commit 6d83ad5，HX-C008）+ **持久化层 678 对齐**（IM_SCHEMA/多租户 tenant.db/cursor=_helix_monotonic）+ **merge main（ADR-010 共享 HTTP/WS client）**（fcf3222）均落地；交付 crate 全测绿 + 真 Go :8065 直连冒烟通过（握手/increment/0 断连）。详见 `docs/过会方案/helix-sync-v2-设计.md`（合并自 real-go-wire + 完成-spec）。

> **2026-06-17 进展**：sync-v2 **Phase C（C1 echo 对账 / C2 phantom+离线落库+readBits / C3 mutation 事件+im:__bus__）+ B2 冷启动自愈** 实现完成（commit `20334d2`，单测+对抗审+`gate.sh` 7 闸门全绿）。详见 `crates/helix-im/docs/sync-v2-能力与缺口状态.md`。

**✅ 已完成（2026-06-17）**
- [x] **sync-v2 数据路径真 Go 端到端验证通过**：真消息 curl→Go post→WS `post` 帧→parser 认 post→gate apply→message 表落真实内容行 + temporary_id↔server_id 对账 + read_bits + cursor 推进 + post:received DomainEvent。**三层 blocker 全清**（grpc-deviceId / sync 风暴-BoundedSpawner 并发闸 / WS-post-action-parser）。详见 `crates/helix-im/docs/sync-v2-能力与缺口状态.md`。
- [x] golden fixtures 真帧回放（`tests/fixtures/real_go_frames/` + golden_replay_test，防 post/posted 同类回归）；im_trace→Tick 剧本完整工具链仍 P1。

**P0（M2 收官阻塞）**
- [x] 最小 native 宿主 bin（`crates/helix-host-cli`）+ transport 路由表（真 Go 冒烟跑通）
- [x] **UC-1 前置 4 块**（panel-review 2026-06-12 揪出）：① hello 握手 ✅ · ② B4 event_seq fallback ✅ · ③ B5 phantom+离线落库 ✅（`20334d2`）· ④ echo→reconcile ✅（`20334d2`，handle_echo 激活）
- [x] B2 increment_end 冷启动自愈（`20334d2` heal_incomplete_backfill_once）
- [x] UC-1 数据路径真 Go smoke ✅（2026-06-17 真消息端到端验过，见上「已完成」）
- [ ] Tauri 四接缝接入 + 灰度开关（含 B7 store:\* 对齐、B3 readBits）
- [x] `helix-driver-host` shared HTTP/WS client 边界阶段落地：native/ffi Rust 侧只留薄壳，业务模块不拥有网络 client，登录鉴权只作为 `HostNetworkConfig` + header/token 配置输入（剩余：真 Go smoke、WS reconnect/backoff、proxy/TLS/cert knobs、旧 callback fallback 完整接线）

**P1（质量与性能）**
- [x] E5 monotonic_upsert DDL 出热路径（建表挪到 open()，monotonic_upsert 仅剩单 INSERT；回归锚 test_monotonic_table_created_at_open_e5）
- [x] E6 gate buffer 上限+主动 too_long（MAX_GATE_BUFFER=16384，超限清 buffer + emit too_long + sync 重拉，cursor 不回退；回归锚 ×2）
- [x] E7 broadcast lagged 可感知（接收侧 EventReceiver/RecvOutcome::Lagged 显式上报；纠正承压评估 emit 侧定位错误——send 侧感知不到 lagged；回归锚 ×2）
- [ ] B6 UpsertSpec conflict 列语义文档
- [ ] golden fixtures 工具链（im_trace 导出 → Tick 剧本）

**P2（性能尾巴 + 工程化）**
- [ ] E8 timer 反向索引 · E9 热路径分配收口（HX-C005）——含 **a0595607 遗留**：`helix-driver-host` batch_upsert SQL 按 (table, col_count, has_conflict) 形状缓存（monotonic cursor SQL 已 const 化，B1 修复内）
- [ ] AI 工程化四件：CI 双端 matrix / clippy deny / per-crate CLAUDE / public-api gate（§四）；public-api 基线含新增 `SqlValue::Blob`（M2）
- [ ] 四能力 feature 拆分（§7 技术债：driver-native optional dep）
- [~] bench 常态化（criterion 化 stress 三件套，数字进 CI 留痕）——**stress-verify 2026-06-15 REJECT 落实**：① criterion 严格基准**全链路覆盖完成**（median/CI/离群，回归判据 median>2x）：storage（`host_storage` 5000 行 5.16ms·39×100 6.08ms·monotonic cursor 28.5µs）+ **engine 泵**（`engine_loop/pump_roundtrip_1k` 40.2ms ≈ 24.9K roundtrips/s）+ **IM 乱序同步**（`gate/backfill_5000` 8.0ms ≈ 624K events/s·`send/echo_reconcile_1000` 2.39ms ≈ 418K/s）+ im_step ② 并发 parity 改「串行会超时」强证明（storage_conformance）。**仅剩**：bench 数字进 CI 留痕（待远端 GitLab runner）
- [ ] FFI MAJ-4：C-ABI 导出层 `spawn_blocking` 包裹落地（HX-C006/ADR-005，当前 todo）
- [x] **背压根治**（2026-06-17）：Effect `Http`(必达)/`HttpFire`(可丢自愈) 二分 + driver 统一 `BoundedSpawner`（Persist/Http/HttpFire 共用，翻默认 Block 传导回源、DropOldest 只给 HttpFire）；**跨模块公平性 tripwire 待第二业务模块接入再做**（不预建调度器，见 driver CLAUDE.md「已知缺口」）
- [ ] **双 `PortError` enum 统一**（tick.rs:56 vs error.rs:51 待合一——折叠自已删稿「完美过会指南」技术债）
- [ ] **多模块并发写实测**（单写者池在多业务模块下零实测——折叠自「并发承压评估」开放项；承压稿已移 `docs/reports/`）

**已销账**：A 全部 · B1 · C1-C4 · E1-E4 · **E5-E7** · M1 全部 · M2 的 driver/im 主体。

---

## 三、最终验收 Checklist（全部 machine-checkable，过一项勾一项）

**结构闸门（每次 merge 必跑）**
- [ ] `cargo check` 双端产品目标（core/im/driver-native/driver-ffi）= 0 error
- [ ] `cargo test --release` workspace 全绿（当前基线 85，只增不减）
- [ ] `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` = 空
- [ ] `grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ | grep -v '//'` = 0
- [ ] harness 索引不变量：CLAUDE.md 表行数 == `ls docs/harness/HX-C*.md | wc -l`
- [ ] （落地后）`cargo public-api -p helix-core` diff 对 allowlist = 0

**行为闸门（承压基线，压测三件套）**
- [ ] proptest 乱序不变量 5 条绿（cursor 单调 / 不丢消息 / 确定性）
- [ ] 10k 裸灌不死锁 + graceful drain 后查库全落盘
- [ ] batch_upsert 5000 行 < 100ms（写放大 ~1.0x）

**M2 联调闸门（真连 Go）**
- [ ] UC-1：发消息 → 0 等待上屏 → echo 对账「已送达」→ 库中 id 已换正式 id
- [ ] UC-2：断 WS 30s → 重连 → deadQueue 续联或 sync 补齐，消息无洞
- [ ] UC-3：模拟离线 >500 事件 → 自动续拉至 no_change，cursor == 服务端 max seq
- [ ] UC-4：多 channel 批量 sync，置顶优先
- [ ] Tauri 灰度：Angular `git diff` = 0；开关回退 < 5s；同一 DB 双引擎数据互认

**M3 追加（Flutter FFI）**
- [ ] 同一 Tick 剧本 native vs FFI 输出 byte-level 一致（可回放跨端验证）
- [ ] Flutter 端 HTTP/WS 由 Rust shared host driver 执行；Dart 侧无业务 fetch/WS/sync/cursor/echo 逻辑
- [ ] FFI Flutter 冒烟 + abort 语义验证
- [ ] `helix-driver-host` storage conformance 测试覆盖 native/ffi 共享 SQLite 语义

---

## 四、AI 工程化对照 Bun（✅ 已全部闭合 — 2026-06-12 落地 / 2026-06-16 销账核实）

> Bun 的设计哲学：**把工程约束嵌入工具链（clippy/hooks/CI），让 AI 的错误选择触发自动反馈，而不是等 review**。
> **状态：原 7 项差距已全部闭合**。本节保留作历史对照与证据索引，**勿再当待办引用**。

| 层 | Bun 机制 | Helix 落地（✅ 已闭合，证据） |
|---|---|---|
| 护栏 | clippy.toml deny-list | ✅ 根 `clippy.toml` deny `Instant::now`/`SystemTime::now`/`thread::spawn`/`println`/`eprintln`（带 reason）；`helix-core`/`helix-im` Cargo.toml `[lints.clippy] disallowed_methods=deny`+`unwrap_used=warn`；driver 三 crate allow（ambient 合法落点） |
| 验证 | CI 多 job 闸门链 | ✅ `.gitlab-ci.yml`（非 GitHub）`gate` stage（双端 check + cargo tree 零运行时 + grep 零业务 + test）+ `api-surface` stage（public-api gate）；`scripts/gate.sh` + `install-hooks.sh` pre-push |
| 指令 | 嵌套 CLAUDE.md ×N（就近继承） | ✅ 7 crate 全配 per-crate CLAUDE.md（core/im/driver-{host,native,ffi,wasm}/host-cli；后两个 2026-06-16 补齐）+ `.claude/rules/`（runtime-agnostic-core / expert-dispatch） |
| 流程 | skills/commands（pattern 即代码） | ✅ `.claude/workflows/` 3 可重放脚本（expert-panel-review / dual-track-fix / stress-verify）+ `/helix-new-module`·`/helix-replay-test`·`/helix-stress-verify` skills + 7 张 HX-C 坑卡 |
| 锁定 | toolchain pinning | ✅ `rust-toolchain.toml` + `rustfmt.toml` |
| 参照 | Zig-as-reference（行为真源） | ✅ root CLAUDE.md §4.5「现网 `cses-client/src-tauri/.../im` 对应 handler 是 source of truth」+ ADR |
| 验证 | bench + 无数字性能主张自动拒 | ✅ criterion 全链路覆盖（storage/engine 泵/IM 乱序 gate，§二 P2 销账）+ `stress-verify` workflow（无数字不收） |

**结对齐（一并已具备）**：结构化 commit + 模块化 tag、对抗 review（4 专家 workflow）、回放测试基建、待办清单销账制——全在。

> ⚠️ **销账记录**：本表原为 2026-06-12 之前的*差距清单*；闭合后一度未销账，2026-06-16 sync-v2 期间被误当待办引用一次（主对话建议「并行补这四件」被用户当场识破为重复劳动）→ 即时销账标 ✅。教训：差距表闭合后必须立即翻面为「已闭合+证据」，否则成误导性文档残留。
