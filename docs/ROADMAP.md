# Helix 细粒度 Roadmap · TodoList · 验收 Checklist

> 2026-06-12 · 基于：技术过会方案 §6（M0-M4）+ 评审 D 系调整（+3 周）+ 实际进度对账。
> 2026-06-15 更新：按 ADR-006/ADR-010，网页端/wasm 暂停维护；产品化路线只保留 PC Tauri + Flutter 手机端；HTTP/WS client 与 Storage 一样走 shared Rust host driver，前端只负责命令、事件与渲染。
> 2026-06-16 更新：shared HTTP/WS client 代码边界已阶段落地到 `helix-driver-host`；native/ffi Rust 侧复用同一套 client 与 header registry。真 Go smoke、WS reconnect/backoff 与 proxy/TLS/cert knobs 仍按后续联调项推进。
> 2026-06-18 更新：**M2 ~90% 完**（native 真 Go 联调 + sync-v2 全链路 + shared host I/O + WS registry + 背压根治 + 基座最优性/北极星双 review 收口全落 main）；**M2 唯一剩余 = Tauri 四接缝接入（D1-3 接缝①②）**。**M3：F1 主体 ✅ / F2·F3 ✅ / F4 ✅ 端到端接通**。
> 2026-06-18 更新②：**M3-F4 完成并 merge dev-zlc（c229531）**。架构 pivot：原计划 Dart-host（Dart 喂帧 + `on_domain_event`→Dart Stream）→ **用户拍板 C++-host 直连**（cpp→rust→cpp：Rust 持 WS、事件批回 C++ vtable、Dart 只在 UI 层收 C++ 转发）。pump 核下沉 `helix-driver-host`，native(1119→168)+ffi 共用薄壳 **parity by construction**。八轮 branch drift-review 判 light/0 blocking。剩 F4.5 Flutter 真机冒烟 + F5 跨端 byte-level 验收。
> 当前位置：**M2 收尾（Tauri 四接缝）+ M3-F4 ✅，下一站 F5 跨端验收 / Flutter 冒烟**（M1 全 + M2 主体 + M3 F1 主体/F2/F3/F4 已完）。
> 2026-07-08 对账：本文件是**活账 + 验收清单**，不是最近所有工程进展的流水账。近期开出的 OTel/trace scope、多租户本地 DB、rich/file upload 等能力必须以各自 gate / reviewer / collector 证据为准，未自动等价为本 Roadmap 绿勾。当前机器事实：双端 `cargo check` PASS；本机未安装 `cargo-public-api`。
> 2026-07-13 更新：Web 仍是产品目标，但原 **W0 浏览器探针 → W1-W4 Web 壳/缓存**执行路线已撤销。当前先拍板新的 Rust 架构：三端统一业务门面、PC/Flutter 默认 host 能力、Web 平台适配能力，以及 local-first/remote fallback 与 WS 物理连接/业务协议边界；拍板前不实施新 Web driver。历史 `helix-driver-wasm` 继续归档。
> 2026-07-14 更新：用户拍板新的三端 Rust 业务门面与平台适配设计，当前决策见 [ADR-014](./harness/decisions.md)。不新增共享大 crate：先收口 `helix-im::client_api` 与异步 query 真源，再缩窄 core WS port，最后建立独立 `helix-driver-web`。PC/Flutter 保持 shared host 默认实现；Web 首版为 Rust 内存 session store + Browser Fetch + 浏览器物理 WS bridge。
> 2026-07-14 A1：`helix-im::client_api` 已成为 alias、payload normalization、capabilities 与 `AppCommand` 构造真源；`mobile_contract` 仅兼容 re-export，FFI 已删除本地业务构造逻辑。native 当前没有 public-name/payload/Tauri invoke 边界，本阶段不凭空增加 wrapper，真实入口出现时直接复用共享门面。
> 2026-07-15 A4/A5：`helix-driver-web` Rust 平台壳、最小浏览器 glue、正式 Web CI/HX-C003 与历史 wasm 清理已落地；Browser Fetch、page-session store/Disabled、浏览器 WS bridge、timer、有界 event batch v2 与 WASM API 均通过构建/测试。真实 Chromium + final WASM + 本地 Go `:8065` 已验证 query URL 鉴权、hello、local-first/HTTP fallback、send/echo 与断连恢复。

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

### AI 工程化硬化（已闭合，见 §四；本段保留为历史对照）

当前 CI 是 native + FFI + Web Rust 三端门：Web 同时执行 native test、wasm final build、双目标 strict clippy 与 host runtime 依赖泄漏守护；浏览器 E2E 仍由独立环境执行，不能用 WASM build 伪造真链路绿色。

- [x] **F1 Flutter FFI API 最小面** ✅：C++-host 直连面定稿——`create_with_network(vtable)/destroy/set_header/remove_header/helix_command` ✅ + `start`(pump 内 register+start)✅；**`on_ws_frame`/`on_http_response` 已删**（C++-host 模型 Rust 自持 WS/HTTP，入站走 `SharedWsClient::with_inbound_tick`）；`helix_query` 仅保留 capabilities/version 元信息，业务查询统一经 `helix_command` + 批量 DomainEvent；剩余验收放入 F5 Flutter demo / native-vs-FFI parity
- [x] **F2 host storage 复用** ✅：`StorageOp` 语义与 native 单一真源对齐，shared conformance 覆盖；**pump 直接用 `HostStorage`**（F4 后 `FfiStorage` 冗余薄壳已 git rm）
- [x] **F3 shared HTTP/WS client + header API** ✅：pump 用 `SharedWsClient`(with_inbound_tick) + `FfiHttpRequester` + `helix_set_header`/`helix_remove_header`（F4 后 `FfiTransport`/Callback 模式已删，Rust 自持 WS/HTTP）
- [x] **F4 engine 接线 + 事件批回 C++**（C++-host 直连 · merge c229531）✅：pump 核下沉 host，FFI 薄壳端到端接通——命令真跑 + 事件批回 C++ vtable。拆解见下 ↓
- [ ] **F5 跨端一致性验收**：同一 Tick 剧本 native vs FFI 输出 byte-level 一致 + schema/migration conformance + shared client 行为验收

#### ▶ F4 实装拆解（FFI 第二端 engine 接线 · 2026-06-18 启动 · ✅ 完成 merge c229531）

> **启动前现状（已解）**：FFI 无 engine_loop——`helix_on_ws_frame`/`on_http_response` 拷贝 buffer 后 `// TODO` 不喂 engine；无 command 入口；核心缺口 = 没有驱动 ExecutionShell 的事件泵（native 有 `engine_loop.rs` 1119 行）。
> **落地结果**：不是「FFI 自写一份泵」，而是**把 native 泵核下沉 `helix-driver-host`**（采纳 F4.1 推荐），native+ffi 都成薄壳共用 `run_engine_loop`——parity by construction，根除两套泵漂移。**架构 pivot**：C++-host 直连下 Rust 自持 WS，入站不再经 `on_ws_frame`（已删）而走 `SharedWsClient::with_inbound_tick`；事件不回 Dart Stream 而**批回 C++ vtable**，由 C++ 转发 Dart UI。

| 子任务 | 交付 | 验收 | 风险/依赖 |
|---|---|---|---|
| **F4.1 engine 泵归属决策** ✅ | **抽取共享（采纳推荐）**：pump 核（`run_engine_loop` 泛型壳 over Storage/Http/EventSink/FrameSender/Clock + `BoundedSpawner` + `dispatch` + `run_http_envelope` + `TimerRegistry` + `consume_effects`）下沉 `helix-driver-host`；native `engine_loop` 1119→168 薄壳复用 | ✅ native 重构**零回归**（cargo test 64 全绿 + stress 3+4）；接缝 = `EngineDeps` + `BatchSink` egress 钩子 | 🔴 已解：连接管理留各端薄壳（native broadcast / ffi vtable），泵核共享 |
| **F4.2 FFI engine runtime + Tick 入队** ✅ | `helix_create_with_network` 起专用 std::thread + current_thread runtime(`.enable_all`) + `register(ImModule)` + `start()`；`helix_command`→`Tick::Command` ✅；**入站改 `SharedWsClient::with_inbound_tick`→`Tick::Inbound`**（C++-host 模型 Rust 自持 WS，`on_ws_frame`/`on_http_response` 已删）；HTTP/PortReply 走 host dispatch internal | ✅ e2e `command_emits_domain_event_through_batch_callback` 命令→step→Effect 通；core 单线程跑（GOAL §1.3）| current_thread（不在 Dart isolate，C++ host 自管线程）|
| **F4.3 事件批回 C++（原"回 Dart Stream"已 pivot）** ✅ | `FfiEventSink impl EventSink`（经 host `BatchSink::flush` 钩子）：每 step 攒本批 Emit→单 buffer→**一次** `CppHostVtable::on_domain_events_batch`（HX-C007，绝不逐条）；C++ 再转发 Dart UI | ✅ 批回调 e2e + 批 wire layout(len-prefixed)单测；egress 在 host 参数化、core EventSink 零改 | flush 钩子 host 非 core（HX-C001 边界）|
| **F4.4 panic=abort + spawn_blocking 终核（MAJ-4）** ✅ | `panic="abort"` 最终 cdylib ✅；pump 同步阻塞经 host `HostStorage` spawn_blocking 继承；无新增裸阻塞 tokio worker；pump 线程 panic 在线程边界隔离 | ✅ clippy ffi 0 警告 + 边界测试 + destroy join 不挂 | HX-C006 全 HOLD（八轮 drift-review 复核）|
| **F4.5 Flutter demo 冒烟 + 跨端一致（= F5 起点）** [~] | cbindgen 生成 `helix.hpp` ✅（C++ opaque + `CppHostVtable` + 6 导出）；Rust extern"C" vtable e2e ✅。**剩**：真 Flutter app `dart:ffi`→C++→Rust 冒烟 + 同一 Tick 剧本 native vs FFI byte-level 一致 | helix.hpp 已交付；**Flutter 真机冒烟 + 跨端 parity 测试待做（F5）** | 依赖真机环境 |

**关键路径**：F4.1–F4.4 ✅ 端到端接通并 merge（c229531）。**采纳「抽取共享」而非「FFI 自写泵」**——native+ffi 共用 host `run_engine_loop`，parity by construction。**剩 F4.5/F5** = Flutter 真机冒烟 + native↔FFI byte-level 跨端一致验收。**headless 守卫兑现**：全程 core/helix-im 零改（cargo tree -p helix-core 零运行时复证），改动落 `helix-driver-{ffi,host,native}`；同一份 core 跑 ffi 零改 ✅。

### M4 —— Web 产品化（ADR-014 · A1-A4 已完成）

> **边界**：Web 复用 `helix-core + helix-im` 的业务门面和状态机；PC/Flutter 的完整 SQLite/SharedWsClient 不搬进浏览器。旧 W0→W4、浏览器 probe、Dedicated Worker/IndexedDB 预设均继续失效。详细决策见 [ADR-014](./harness/decisions.md)，文件/类型/验收以本节活账为准。

- [x] **A1 统一 `helix-im::client_api`**：alias/normalization/canonical name/capabilities/`AppCommand` 构造已提升为共享真源；`mobile_contract` 保持兼容 re-export，FFI 只余错误码映射。native 尚无同形公开入口，未新增空壳 wrapper。〔验收〕共享/兼容路径字节一致；known borrowed、fallback owned、非法 payload 与 FFI ABI 均有回归测试；core/im/FFI 全量测试 0 failure。
- [x] **A2 查询旁路收口 + local-first**：以现有 `helix-im::query` 异步路径为唯一真源，引入 Durable/Session/Disabled 与 Complete/Partial/Miss/Unsupported，远端 fallback 仍产 `Effect::Http`。〔验收〕11 条端到端回放 + 8 条 coverage 单测覆盖本地直出、回源合并、缓存顺序、失败、coverage 反证、会话隔离与陈旧行清理；FFI `mobile_query` 与同步 Storage 旁路已删除，业务查询统一经异步命令与事件批次。
- [x] **A3 WS port 缩窄**：core 只暴露 send-only `FrameSender`；host engine/dispatch 只依赖发帧，`SharedWsClient`/`NativeTransport` 以具体类型管理 connect/recv/close，reconnect 留在 host；旧 core `Transport` 已删除。〔验收〕只实现 send 的 dyn-safe 契约、精确 TransportId/bytes 路由、动态注册后发送、native 受控本地 WS 的三类 Tick/reader 内部顺序、两份独立 shell 对相同**注入**生命周期 Tick 的逐步业务输出一致；core 零 runtime。这里不声称 native/FFI 物理生产者已 parity。
- [x] **A3.1 真实 WS 生命周期收口**：`SharedWsClient::connect` 只完成握手并返回一次性 activation；host 动态注册先 insert sender 再回 ack，native 预填表与 FFI 初连/重连均在 sender 可路由后发布 Connected，随后才放行 reader。reader/显式 close 共用每连接 `Prepared → Active → Finished` 状态，remote-close + cleanup-close 只产一次 Disconnected。〔验收〕host Connected 即 Send 命中注册 sender；native 本地 WS 严格 `Connected → Inbound → Disconnected` 且 cleanup 无尾随 Tick；FFI 真实 pump 两代连接严格 `Connected → Inbound → Disconnected → Connected → Inbound`。
- [x] **A4 新建 `helix-driver-web`**：纯 Rust 实现 Browser Fetch（完整 body、timeout、shutdown abort）、有界 page-session store、ExternalWsBridge、browser timer、有界 event batch v2 与 WASM API，不写前端页面；`helix-im` 注册机制由不支持该 wasm target 的 `linkme` 单轨迁为 `inventory`，Web final artifact 初始化 WASM constructors，并以显式链接锚 + 28 handler fail-closed 校验防 wasm-ld 静默裁注册项。HTTP outcome 业务策略在 `helix-im` 单一持有，host/Web 经 observer 复用；所有 JS 轮询输出队列都有硬上限，event batch 用 gap/drop 计数报告丢弃。〔验收〕`wasm32-unknown-unknown` build 通过，相关六 crate 全测通过，同 Tick 剧本三端 parity 通过；Web 未复制业务 fetch/WS parser/sync/cursor。
- [x] **A5 真链路与清理**：最小平台 glue、hello/收帧、local-first/HTTP fallback、文本 WS 出站、正式 Web gate/HX-C003、历史 `helix-driver-wasm` 与 FFI `mobile_query` 删除均已完成；真实 Chromium + final WASM + 本地 Go `:8065` 已验证 URL `cookieId` 鉴权、HTTP 会话身份、send→echo 与断连恢复，Disabled store 模式全程零 runtime/adapter issue。〔验收〕浏览器矩阵和真 Go E2E 通过，且继续复用 A3.1 的真实 adapter Tick 顺序/去重合同。

---

## 二、TodoList（活账，按优先级）

> **2026-06-16 进展**：B1/B2 主体 + **B3 sync 真 wire**（commit 6d83ad5，HX-C008）+ **持久化层 678 对齐**（IM_SCHEMA/多租户 tenant.db/IM cursor=`channel_event_cursor`，`_helix_monotonic` 仅 core 通用原语）+ **merge main（ADR-010 共享 HTTP/WS client）**（fcf3222）均落地；交付 crate 全测绿 + 真 Go :8065 直连冒烟通过（握手/increment/0 断连）。详见 `docs/过会方案/helix-sync-v2-设计.md`（合并自 real-go-wire + 完成-spec）。

> **2026-06-17 进展**：sync-v2 **Phase C（C1 echo 对账 / C2 phantom+离线落库+readBits / C3 mutation 事件+im:__bus__）+ B2 冷启动自愈** 实现完成（commit `20334d2`，当时 gate 全绿）。当前 `scripts/gate.sh` 是 10 步默认门，覆盖双端 check/test、HX-C001、trace、业务边界、harness、FFI 与 public API。当前 UC 链路口径见 [`前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](../crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md)。

**✅ 已完成（2026-06-17）**
- [x] **sync-v2 数据路径真 Go 端到端验证通过**：真消息 curl→Go post→WS `post` 帧→parser 认 post→gate apply→message 表落真实内容行 + temporary_id↔server_id 对账 + read_bits + cursor 推进 + post:received DomainEvent。**三层 blocker 全清**（grpc-deviceId / sync 风暴-BoundedSpawner 并发闸 / WS-post-action-parser）。机制见 [`helix-sync-v2-设计.md`](./过会方案/helix-sync-v2-设计.md)，当前 UC 缺口见前端 UC 投影清单 §3-§4。
- [x] golden fixtures 真帧回放（`tests/fixtures/real_go_frames/` + golden_replay_test，防 post/posted 同类回归）；im_trace→Tick 剧本完整工具链仍 P1。

**P0（M2 收官阻塞）**
- [x] 最小 native 宿主 bin（`crates/helix-host-cli`）+ transport 路由表（真 Go 冒烟跑通）
- [x] **UC-1 前置 4 块**（panel-review 2026-06-12 揪出）：① hello 握手 ✅ · ② B4 event_seq fallback ✅ · ③ B5 phantom+离线落库 ✅（`20334d2`）· ④ echo→reconcile ✅（`20334d2`，handle_echo 激活）
- [x] B2 increment_end 冷启动自愈（`20334d2` heal_incomplete_backfill_once）
- [x] UC-1 数据路径真 Go smoke ✅（2026-06-17 真消息端到端验过，见上「已完成」）
- [ ] PC native 四接缝接入 + 灰度开关（含 B7 store:\* 对齐、B3 readBits）：直接以 `cses-client` Tauri 壳作为现役消费端，接入 command/host observation、HostEffect、批量 InteractionFrame 与 ack/resync；投影录放装饰器采用 `crates/helix-driver-host/docs/external-host-instrumentation-recipe.md` 的 `BatchSink`/`RecordingSink` 机制，不依赖外部验证仓作为首落地。
- [ ] **InteractionProtocol/v1 多端统一 UI 交互（ADR-015）**：按 `docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md` 落地 SurfaceModel、ViewNode/ViewMutation、ActionBinding、UiDirective、surface/epoch/revision/ack/resync；先纵切 G-01，再迁移窗口/回复、已读和群治理。实施仓库固定为 `helix`、`cses-client`、`mobile-qucik-c++`，`cses-im-server` 只在真实合同缺口时条件介入；明确排除 `loopforge-tauri-im` 与 `pd/message`。各端不得保留 temporaryId 对账、窗口排序、未读/权限或 fallback 业务 reducer；具体 Component/Widget/DOM 映射仍在平台 shell。
- [x] `helix-driver-host` shared HTTP/WS client 边界阶段落地：native/ffi Rust 侧只留薄壳，业务模块不拥有网络 client，登录鉴权只作为 `HostNetworkConfig` + header/token 配置输入（剩余：真 Go smoke、WS reconnect/backoff、proxy/TLS/cert knobs、旧 callback fallback 完整接线）

**P1（质量与性能）**
- [x] E5 monotonic_upsert DDL 出热路径（建表挪到 open()，monotonic_upsert 仅剩单 INSERT；回归锚 test_monotonic_table_created_at_open_e5）
- [x] E6 gate buffer 上限+主动 too_long（MAX_GATE_BUFFER=16384，超限清 buffer + emit too_long + sync 重拉，cursor 不回退；回归锚 ×2）
- [x] E7 broadcast lagged 可感知（接收侧 EventReceiver/RecvOutcome::Lagged 显式上报；纠正承压评估 emit 侧定位错误——send 侧感知不到 lagged；回归锚 ×2）
- [x] **B6 UpsertSpec conflict 列语义文档**（commit 9009b1f，经 `feat/p2-quality` 合入 main）：`core/ports/storage.rs` batch_upsert doc 补 ON CONFLICT 三形态 + exclude 守卫语义
- [ ] Interaction golden fixtures 工具链（im_trace/UiEvent/HostObservation/EffectResult 导出 → Tick 剧本 → InteractionFrame）：Helix 生成并维护 canonical fixtures，`cses-client` native 与 `mobile-qucik-c++` FFI 使用同一批 fixture 做字节等价 apply/replay；真实录放仍复用 `RecordingSink`，但不依赖排除范围内的仓库生产 golden frame。

**P2（性能尾巴 + 工程化）**
- [x] **E8 timer 取消已 O(1)**（查证 no-op · 2026-06-23 · `feat/p2-quality`）：core `timer_map.peek/remove(id)` + host `timer.cancel` 均 HashMap O(1)；全仓 grep 无 retain/iter O(n) 取消扫描——**本就无活**，非「待加反向索引」（ROADMAP 旧描述误导）
- [x] **E9a cursor 推进零分配** ✅（`615f2f3`，HX-C005 回正）：monotonic cursor 推进 SQL const 化（`Cow::Borrowed`）+ now_ms 条件化 + 未读 guarded_bump bump_delta 绑参
- [x] **E9b 热路径分配尾巴**（HX-C005 · commit 7661da4，经 `feat/p2-quality` 合入 main）：`helix-driver-host/upsert_sql.rs` thread-local 按 (table, col_count, has_conflict) 形状缓存 batch_upsert SQL，去 per-call 分配，零行为变更 +2 单测（driver-host 22 测全绿）
- [x] AI 工程化四件：CI 双端 matrix / clippy deny / per-crate AGENTS / public-api gate 已闭合（证据见 §四）。本机若未安装 `cargo-public-api` 会跳过本地 diff，CI `api-surface` job 与 `docs/api-baseline/helix-core.txt` 是当前公开面基线入口。
- [ ] 四能力 feature 拆分（§7 技术债：driver-native optional dep）
- [~] bench 常态化（criterion 化 stress 三件套，数字进 CI 留痕）——**stress-verify 2026-06-15 REJECT 落实**：① criterion 严格基准**全链路覆盖完成**（median/CI/离群，回归判据 median>2x）：storage（`host_storage` 5000 行 5.16ms·39×100 6.08ms·monotonic cursor 28.5µs）+ **engine 泵**（`engine_loop/pump_roundtrip_1k` 40.2ms ≈ 24.9K roundtrips/s）+ **IM 乱序同步**（`gate/backfill_5000` 8.0ms ≈ 624K events/s·`send/echo_reconcile_1000` 2.39ms ≈ 418K/s）+ im_step ② 并发 parity 改「串行会超时」强证明（storage_conformance）。**仅剩**：bench 数字进 CI 留痕（待远端 GitLab runner）
- [~] FFI MAJ-4：pump 路径同步 SQLite 经 host `HostStorage` spawn_blocking(×7)继承 ✅（HX-C006/ADR-005）；`helix_query` 同步业务读旁路已删除，业务查询复用 pump 异步主路径；**剩** Flutter 真机 demo 与同一 Tick 剧本 native-vs-FFI byte-level parity
- [x] **FFI pump 连接-插表时序类型化**（drift-review wf_efcf1ca4-759 #4）✅ **已彻底解**（commit `34c54bd`，2026-06-22；A3 于 2026-07-14 收窄 port）：**注册通道**——连接期 transport 独占在 loop 外后台跑 connect（裸 ws·不在表），连成功才经 `transport_rx` channel 投进 loop 私有 `mut transports` 表 → 连接期 `transports.get()=None` 走 dispatch warn 兜底（**类型层不可见**，不再靠 hello-gated 时序不变量）。当前 core 只要求 `FrameSender::send(&self)`；`recv/close` 与 `connect` 都是具体连接类型的方法，`TransportTable` 为 `HashMap<TransportId,Arc<Tr>>`；biased select 第二臂 insert+continue（守 pump 5 不变量）。**不新增 per-Send 锁、不回退 create 非阻塞**。验：lifecycle_net_decouple 3/3 + 5-crate+wasm core check + driver 全测绿。
- [x] **背压根治**（2026-06-17）：Effect `Http`(必达)/`HttpFire`(可丢自愈) 二分 + driver 统一 `BoundedSpawner`（Persist/Http/HttpFire 共用，翻默认 Block 传导回源、DropOldest 只给 HttpFire）；**跨模块公平性 tripwire 待第二业务模块接入再做**（不预建调度器，见 driver AGENTS.md「已知缺口」）
- [~] **双 `PortError` enum 改名消歧**（~~统一~~ 误诊已纠 · 查证 2026-06-23）：tick.rs:56 与 error.rs:51 **语义独立非重复**（`error.rs`=携 String 的 thiserror 边界错误；`tick.rs`=带状态码数字的重试分类 Timeout/Http(u16)/Storage(u32)/Network）——**折叠是回归**，正解是改名（tick::PortError→RetryClass/PortErrCode）消歧；干净改名须碰 helix-im 测试故 HOLD
- [ ] **多模块并发写实测**（单写者池在多业务模块下零实测——折叠自「并发承压评估」开放项；承压稿已移 `docs/reports/`）

**已销账**：A 全部 · B1 · C1-C4 · E1-E4 · **E5-E7** · M1 全部 · M2 的 driver/im 主体。

---

## 三、最终验收 Checklist（全部 machine-checkable，过一项勾一项）

> **2026-07-08 对账**：本节是最终验收清单，不等于当前全绿状态。最新同跑证据：`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` PASS；本机 `cargo-public-api` 未安装，公开面 diff 未本机验证。

**交付闸门（每次 merge 必跑）**
- [x] `cargo check` 双端产品目标（core/im/driver-native/driver-ffi）= 0 error（2026-07-08 本机 PASS）
- [ ] `cargo test --release` workspace 全绿（以当前 `scripts/gate.sh`/CI 输出为准；本轮未跑，不能声称基线全绿）
- [ ] `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` = 空
- [ ] `grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ | grep -v '//'` = 0
- [ ] harness 索引不变量：AGENTS.md 表行数 == `ls docs/harness/HX-C*.md | wc -l`
- [ ] `cargo public-api -p helix-core` diff 对 allowlist = 0（本机工具缺失时不可声称已验；CI `api-surface` job 为远端入口）

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

**M4 追加（Web）**
- [ ] 三端 command/query 全部经过 `helix-im::client_api`，driver 业务旁路 = 0
- [x] local-first 对 Complete/Partial/Miss/Unsupported 均有可证伪回放
- [x] core WS public port 只保留 outbound frame，仍零 runtime/platform 依赖
- [x] PC/Flutter 继续共用 host SQLite/HTTP/WS；A4 六 crate 全测与 parity 零回归
- [x] Web HTTP 由 Rust Browser Fetch 执行；物理 WS bridge 不解析业务帧
- [x] Web session store 不跨刷新，cursor 不脱离 coverage 单独恢复
- [x] 同一 Tick 剧本 native/FFI/Web 输出同业务 Effect/DomainEvent

---

## 四、AI 工程化对照 Bun（✅ 已全部闭合 — 2026-06-12 落地 / 2026-06-16 销账核实）

> Bun 的设计哲学：**把工程约束嵌入工具链（clippy/hooks/CI），让 AI 的错误选择触发自动反馈，而不是等 review**。
> **状态：原 7 项差距已全部闭合**。本节保留作历史对照与证据索引，**勿再当待办引用**。

| 层 | Bun 机制 | Helix 落地（✅ 已闭合，证据） |
|---|---|---|
| 护栏 | clippy.toml deny-list | ✅ 根 `clippy.toml` deny `Instant::now`/`SystemTime::now`/`thread::spawn`/`println`/`eprintln`（带 reason）；`helix-core`/`helix-im` Cargo.toml `[lints.clippy] disallowed_methods=deny`+`unwrap_used=warn`；driver 三 crate allow（ambient 合法落点） |
| 验证 | CI 多 job 闸门链 | ✅ `.gitlab-ci.yml`（非 GitHub）`gate` stage（双端 check + cargo tree 零运行时 + grep 零业务 + test）+ `api-surface` stage（public-api gate）；`scripts/gate.sh` + `install-hooks.sh` pre-push |
| 指令 | 嵌套 AGENTS.md ×N（就近继承） | ✅ 8 crate 全配 per-crate AGENTS.md（core/im/driver-{host,native,ffi,web,wasm}/host-cli） |
| 流程 | repo-local agent 路由 + skills | ✅ `.agents/` 路由/约束订阅 + `/helix-new-module`·`/helix-replay-test`·`/helix-stress-verify` skills + HX-C 坑卡 |
| 锁定 | toolchain pinning | ✅ `rust-toolchain.toml` + `rustfmt.toml` |
| 参照 | Zig-as-reference（行为真源） | ✅ root AGENTS.md §4.5「现网 `cses-client/src-tauri/.../im` 对应 handler 是 source of truth」+ ADR |
| 验证 | bench + 无数字性能主张自动拒 | ✅ criterion 全链路覆盖（storage/engine 泵/IM 乱序 gate，§二 P2 销账）+ `stress-verify` workflow（无数字不收） |

**结对齐（一并已具备）**：结构化 commit + 模块化 tag、对抗 review（4 专家 workflow）、回放测试基建、待办清单销账制——全在。

> ⚠️ **销账记录**：本表原为 2026-06-12 之前的*差距清单*；闭合后一度未销账，2026-06-16 sync-v2 期间被误当待办引用一次（主对话建议「并行补这四件」被用户当场识破为重复劳动）→ 即时销账标 ✅。教训：差距表闭合后必须立即翻面为「已闭合+证据」，否则成误导性文档残留。
