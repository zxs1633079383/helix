# Helix driver-ffi 全量接管 SPEC — native → 全量 driver-ffi(all UC + HTTP + WS)

> **一句话目的**：把 `helix-driver-ffi` 从当前 ~95% 提升到与 `helix-driver-native` **全量行为对齐**——经 7 个冻结的 C ABI 函数，承载 IM 全部 **88 outbound + 1 im_send_message = 89 出站/写命令**（+ 19 WS live 入站事件 + query 本地读，分列不计入命令面）、完整 HTTP/WS 网络栈、22 投影工厂(21 distinct channel)回流，**手机端(Dart/Flutter)只渲染、所有业务逻辑留在 Rust(helix-core/helix-im)**。本 spec 是 C ABI 面的冻结契约 + Gap 决策 + 分阶段实现/验证计划。
>
> ⚠️ **本文件已过一轮 5-lens 对抗 review 修订（2026-06-25）**：修正了「C++ shim host 层」漏判引发的一串错误——G-1/G-2/G-4 从「P0 Rust 改码」降级为「契约澄清（借出契约正确，shim 稳定化吸收 Dart 异步）」；命令基线 45→88 事实修正；PI-3 状态据实改 ❌。详见各节 review 标注。

---

## 0. 第一性原则

1. **driver-ffi 的职责止于 C ABI**：只导出 `helix.hpp`（cbindgen 生成）+ cdylib/staticlib 符号。它是「平台驱动层」的 FFI 形态，**不含任何业务逻辑**。
2. **所有业务逻辑在 helix-core / helix-im**：命令分发、状态机推进、cursor 推进、投影生产全部在 Rust 核内（sans-IO 纯状态机 + Module 宿主）。driver-ffi 只做：装配 composition root（`register(ImModule)`）、把 C buffer 翻译成 `Tick`、把 Effect/投影批量推回 C 回调。
3. **手机端(Dart/Flutter)只渲染**：UI = Flutter widget 树；逻辑 = 100% Rust。Dart 侧只负责 ① 把用户动作编码成 `(name, JSON payload)` 调 `helix_command`；② 在 `on_domain_events_batch` 回调里解码投影批 → `setState` 重画。**无任何业务判断在 Dart**。
4. **责任边界止于 C ABI；Dart glue 仓外**：本 spec 与本仓 **只交付到 `helix.hpp` + cdylib + ffigen 可消费的 C 头**。Dart/Flutter 工程（`NativeCallable` 包装、`ffigen` 配置、widget）**在仓外**，由 Dart 侧消费本契约，不进本仓 CI 必跑矩阵。
5. **C 符号面不随 UC 增长**（三份分析独立证伪失败确认）：命令是**数据**（name 字符串 + JSON payload），不是 C 符号。新增 UC = 在 helix-im `OUTBOUND_COMMANDS` 增数据条目，**禁新增 `extern "C"` 符号**。
6. **已知第一性原则破例（诚实记账，非「零泄漏已达成」）**：`api.rs:102 trim_end_matches("/api/cses")` 业务 URL 常量泄漏进 driver（违 HX-C001），native+ffi 双端一致泄漏；待下沉 ImConfig / env 注入收口（G-9，M2 前不阻塞 ABI 冻结）。另 storage.rs cursor 特化 SQL 的「性能白名单 vs schema 泄漏」边界待澄清（G-17）。

---

## 1. C ABI 面冻结契约

### 1.1 七函数逐个签名（冻结，不可增）

源 `crates/helix-driver-ffi/helix.hpp` + `src/api.rs`。

> ⚠️ **实现期符号名对齐（spec 与代码出入，实现 Phase 1 必校真名，勿照抄本表理想签名）**：当前实际导出是
> `helix_create_with_network(api_url, api_len, ws_url, ws_len, vtable)`（非本表 `helix_create(vtable, cfg)`）。
> 本表把创建函数理想化为「vtable + config 结构体」形态——**这是目标形态建议，不是现状**。实现时以 `api.rs`
> 真实签名为准；若要收敛到 config 结构体形态需单独决策（不在本轮 P0/P1 范围）。

| # | 函数 | 签名（C ABI 语义） | 职责 | 状态 |
|---|---|---|---|---|
| 1 | `helix_create` | `HelixHandle* helix_create(const CppHostVtable* vtable, const HelixConfig* cfg)` | 装配 core+im、起 pump 专线程、返回 opaque handle（`Box::into_raw`，单 owner） | ✅ 已通 |
| 2 | `helix_destroy` | `void helix_destroy(HelixHandle* h)` | 停 pump、join 线程、`Box::from_raw` 回收（null-check；内部禁新增 panic 路径） | ✅ 已通 |
| 3 | `helix_set_header` | `int32_t helix_set_header(HelixHandle* h, const uint8_t* k, uintptr_t klen, const uint8_t* v, uintptr_t vlen)` | 鉴权头热更新（注入 Authorization 等；覆盖 401 重注链路） | ✅ 已通 |
| 4 | `helix_remove_header` | `int32_t helix_remove_header(HelixHandle* h, const uint8_t* k, uintptr_t klen)` | 移除鉴权头 | ✅ 已通 |
| 5 | `helix_command` | `int32_t helix_command(HelixHandle* h, const uint8_t* name, uintptr_t nlen, const uint8_t* payload, uintptr_t plen)` | **唯一命令入口**：全部 outbound/WS/query/send/reconnect 经此分发（`AppCommand{name,payload}` → `Tick::Command`） | ✅ 已通 |
| 6 | `helix_query` | `int32_t helix_query(HelixHandle* h, const uint8_t* name, uintptr_t nlen, const uint8_t* payload, uintptr_t plen, uint8_t** out_ptr, uintptr_t* out_len)` | capabilities/version 元信息出口；业务查询返回 `FFI_UNIMPLEMENTED` 并改走异步命令 | ✅ 元信息 |
| 7 | `helix_query_free` | `void helix_query_free(uint8_t* ptr, uintptr_t len)` | 释放 `helix_query` 返回的 boxed slice（`Box::<[u8]>::from_raw`，单 owner） | 🟡 配套 stub |

### 1.2 返回码体系

源 `api.rs:31-41`。

| 码 | 值 | 语义 |
|---|---|---|
| `FFI_OK` | 0 | 投递成功 |
| `FFI_NULL` | -1 | handle / 指针 null |
| `FFI_UTF8` | -2 | name 非 UTF-8（payload 不强制 UTF-8，二进制安全） |
| `FFI_INVALID` | -3 | header registry 拒绝 / authorization 格式错（**信息量过低**，见 §7-G12 `helix_last_error` 选项） |
| `FFI_BUSY` | -4 | tick channel 满（**唯一背压信号**，Dart 须 retry-with-backoff） |
| `FFI_UNIMPLEMENTED` | -5 | helix_query stub 当前返回 |

### 1.3 「不随 UC 增长」不变量 + 证据

- **不变量**：C 符号集 = 7，**任意新 UC 不得新增 `extern "C"` 符号**。
- **证据**（三份分析独立确认）：
  - 分析2 grep：`helix_command` 内 `match name` / `if name==` / `is_query` / `outbound` **全空**——零 per-command 分支，纯通用分发（`api.rs:340-381`）。`AppCommand{name: Cow, payload: Bytes}`（`tick.rs:74-78`）仅 name+payload。
  - HTTP/WS 由 Rust 内部持有（`pump.rs` SharedHttpClient/SharedWsClient），**C ABI 不需要 connect/send/recv 符号**——结构性保证面不增长。
  - 证伪失败（§7 论断1）：同步业务返回值 / per-call correlationId / 二进制大附件 / 反向流控 4 个候选反例，**全部可在 payload 数据层解决**（`client_corr` 字段 + 投影回带），不破 ABI。

### 1.4 三条已知 ABI 边界（**必须显式声明，防「不增长=够用」偷换概念**）

1. **无 per-call 同步业务返回值**：`helix_command` 只返回 `i32` 投递码，所有业务结果走 `on_domain_events_batch` 异步回灌。
2. **无 client correlationId**：`AppCommand` 无 client-supplied 配对字段；请求-响应配对（如 optimistic UI 回滚需「这次 send 成功否」）靠 **payload 业务字段反查**（channel_id 等）。若某 UC 硬需配对 → payload JSON 加 `client_corr` 字段 + 投影 emit 回带同字段（数据层，不动 C ABI）。
3. **背压只在入站**：`helix_command` 满返 `FFI_BUSY`（入站唯一背压，Dart 须 retry-with-backoff）；**出站投影无背压**（`on_domain_events_batch` fire-and-forget 无返回值，Dart 自行限速）。（review D17：原「FFI_BUSY 唯一背压」措辞不准——出站根本无流控。）

---

## 2. 指令集合（数据契约 = 用户要的「指令集合」）

> **核心**：命令是**数据**（`name: 字符串` + `JSON payload: Bytes`），**不是 C 符号**。一个 `helix_command` 分发全部。新增命令 = helix-im `OUTBOUND_COMMANDS` slice 数据条目，零 driver 改动。

### 2.1 分类总表

| 类别 | 数量 | name 来源 | payload 契约真源 | 触发的投影事件类 |
|---|---|---|---|---|
| **outbound（HTTP 出站写命令）** | **88** | `OUTBOUND_COMMANDS`（helix-im，registry 真名；**6 文件用 `macro_rules!` 展开**，故 grep 的 45 行 ≠ 真实条数） | `docs/csesapi-migration/full-map/partials/1..4-*.md` + **`docs/csesapi-migration/真机curl真源.md`**（真抓包 body/头 ground truth，优先于源码推断） | `im:__bus__` 投影（信号/透传/控制三类，见 `projection-schema.md`） |
| **im_send_message** | 1 | 专用发送命令 | `真机curl真源.md`（posts/create body — send 无回声真因=体不对，非 self-omit/Pulsar） | 发送者收 `action=post` 回声投影 |
| **im_reconnect** | 1 | 重连控制命令 | 触发 WsRuntime 重连状态机 | 连接态投影（reconnect/online） |

> **命令面合计 = 88 outbound + 1 im_send_message = 89 出站/写命令**（真源拆分 `04-cses-client接入方案.md:84`：`8+17+17+11+35=88`）。**0 个需要 C 符号**——全经 `helix_command(name, payload)` 数据分发。`im_reconnect` 含在控制命令内。

**以下两类分列，不计入命令面**（review P1-2/P1-3 修正：方向/范畴不同）：

| 类别 | 数量 | 说明 |
|---|---|---|
| **WS 入站 handler** | 28 注册（= 19 live + dead/ignored） | `ws/registry.rs` action→handler。**入站**（WS→Rust），与命令（出站 Dart→Rust）方向相反，不是命令 |
| **query（本地读，路径异质）** | 4 个 `is_query` 臂 | **路径不统一**：`im_query_messages_by_channel` / `im_query_dialog_list` → `Effect::Persist(Scan)`→`PortReply`→emit；`im_delete_all_dialogs` → 内存 clear 同步 emit（无 Scan）；`LOAD_OLDER_CONTEXT` → 发 HTTP `postContext` Effect（**需网络**，非纯本地读）。权威「本地读 query」只算 2 个 |

### 2.2 payload 来源指针（消费视角真源，渐进式 Read）

- **真抓包 ground truth（最高优先）**：`crates/helix-im/docs/csesapi-migration/真机curl真源.md` — 现网 cses-client 实抓 6 个 curl（posts/create·makeTopic·revoke·channel/create·member/change·close）逐字 body+头。outbound 命令体**必须对齐此处**。
- **全量 HTTP 清单**：`full-map/partials/1-…posts` · `2-…channel-sync` · `3-…user-misc` · `4-…bot-agent`（124 接口四域 + wire 字段 + 命名陷阱）。
- **WS 推送清单**：`full-map/partials/5--backend-ws-events.md`（19 事件常量 + `event_seq` 注入铁律）。
- **投影 payload 字段集**：`crates/helix-im/docs/projection-schema.md`（C2 冻结，21 emit 工厂 + 配 `tests/projection_schema_test.rs` 强制对账）。

### 2.3 投影事件类（回流，三类）

> ⚠️ **review P1-4 修正**：`im:__bus__ {channel,payload}` 信封是 **native(Tauri) 交付格式**（做在 `helix-driver-native/event_sink.rs::to_bus_envelope`）。**FFI C ABI 边界不产生该信封**——交付的是 length-prefixed 批的**原始 `{"event":..,"data":..}` JSON**（`batch.rs`），信封化（如需要）由仓外 Dart 自己做。详见 §4。

三类（按 `projection-schema.md` 分类，与传输格式无关）：
- **信号类**：状态变更通知（已读、在线、cursor 推进）。
- **透传类**：消息体/频道体直接投影到 UI。
- **控制类**：resync_needed / too_long fallback / 连接态切换。

---

## 3. vtable 契约（Dart NativeCallable）

源 `crates/helix-driver-ffi/src/vtable.rs:33`（`CppHostVtable` `#[repr(C)]`）+ helix.hpp:36-57。

### 3.1 vtable 布局

```c
struct CppHostVtable {
    uint64_t (*now_ms)(void* ctx);                              // 时间注入
    void     (*on_domain_events_batch)(void* ctx, const uint8_t* ptr, uintptr_t len);  // 投影批回灌
    void*    ctx;                                               // host 上下文，单 owner = host
};
```

### 3.2 `now_ms` 契约（**目标：保留 vtable.now_ms 字段，由 C++ shim host 同步填**）

> ⚠️ **review P0-2 修正方向**：上一版误把 host 窄化成「Dart」，于是虚构跨 isolate 难题、要 Rust 自持 SystemTime——**方向反了**。真 host = **C++ shim**（与 Rust 同进程、同步调用，无 isolate 问题）。

- **问题边界**：`now_ms` 在 pump 每 step 同步调（`clock.rs`），**必须同步返回 u64**。**Dart 不能直接填**（`NativeCallable.listener` fire-and-forget 无返回值；`.isolateLocal` 从 pump 专线程调 = UB）。但这只说明 **Dart ≠ host**，不说明要移除字段。
- **目标（非冻结，待实现）**：**保留 `vtable.now_ms` 字段**，由 **C++ shim host 同步提供**（shim 与 pump 同进程，回调线程内同步返回 epoch ms）。GOAL §1.2 确定性注入靠此字段成立——**禁移除字段让 Rust SystemTime 成正路**（那会使 FFI 端比 native 更不确定，方向反）。
- **现状（G-2 待修）**：`clock.rs:38` 现为 `match now_ms_fn { Some(f) => f(ctx), None => SystemTime }`——**已优先调 vtable.now_ms**，SystemTime 仅 fallback。即字段链路已对，**G-2 的「待修」仅是文档把 host 写清是 shim、并在测试用 mock now_ms 跑确定性回放**，非改 ABI。

### 3.3 `on_domain_events_batch` 契约（**维持现网借出契约；shim 稳定化吸收 Dart 异步**）

> ⚠️ **review P0-3 修正（5 人战场，最重磅）**：上一版漏判 **C++ shim host 层**，以为 Dart 直连裸 vtable → 虚构 listener UAF → 要改 ownership-transfer + 加 `helix_free_batch`（**第 8 符号，破 §1.3 七符号冻结**）。**demo 已证伪**（`flutter-demo-pointer.md` 阶段1=done）：架构上 shim 必在中间，Dart 不能把 raw ptr 当 C 函数指针直接消费。

- **回调线程**：在 pump 专用线程触发（`event_sink.rs:72-79`）。
- **buffer 所有权模型（冻结）= 维持现网借出契约**：`ptr..ptr+len` **仅在 `cb` 回调期间有效**，回调返回后 Rust 就地 drop。**这是正确边界，无 UAF。**
- **Dart 异步如何吸收**：**C++ shim 在回调线程内 `malloc + memcpy`** 到稳定堆，再投递给 Dart isolate（`flutter-demo-pointer.md` ①稳定化，已 done）。`helix_shim_free_batch` 是 **shim 层函数，非 Rust 导出**——**不新增 C ABI 符号**，§1.3 七符号冻结完整。
- **Rust 侧零改动**：现网 `event_sink.rs:72-79` 借出契约**保持不变**。G-1 从「P0 改 ownership-transfer」**降级为「契约澄清」**（写清 host=shim、稳定化在 shim）。
- **派生消解（G-4）**：借出模型下编码 buffer 始终 Rust own、回调返回即可复用 → **scratch 复用与所有权无冲突**（原 G-4「互斥」前提不成立，删除）。

### 3.4 ctx 生命周期 + destroy 顺序

- `ctx` 单 owner = **C++ shim host**（非 Dart 直接持），`helix_destroy` 返回前不得释放。
- **destroy 顺序采 demo 已验证 6 步**（`flutter-demo-pointer.md` ptr 生命周期/销毁节）：含 `helix_destroy`（停新回调 + join pump 线程）→ shim/Dart drain 在途批 → **显式接受丢末批**（join 完成后 Rust 不再发新批；shim 侧 close 前 drain 残批，未 drain 完的末批可丢，不阻塞销毁）。
- 借出+shim 模型下末批 buffer 独立于 handle 生命周期 → §3.4 顺序成立（佐证 §3.3 借出契约正确）。

---

## 4. 投影回流契约

### 4.1 批 wire layout

源 `crates/helix-driver-ffi/src/batch.rs:30-46`（小端，单次容量预估避 realloc）：

```
[ u32 count ]                          // 本批投影条数（小端）
repeat count 次:
  [ u32 len ][ payload(len bytes) ]    // 每条：u32 长度前缀 + JSON payload
```

- **decode 零信任**（`batch.rs:51-71`）：截断 / 非法 len → 返回 None（边界输入 `Result`，零 unwrap）。
- **空批防御**：`flush` 空缓冲直接返回（`event_sink.rs:57`）+ pump `owned.is_empty()` 门控——双重防空批跨边界无谓回调。
- **批边界 = HX-C007**：每 step `dispatch_effects` 后**一次** `flush()`，**绝不逐条**过边界。

### 4.2 21 emit 工厂指针

源 `crates/helix-im/docs/projection-schema.md`（C2 冻结）：**22 个投影 emit 工厂 → 21 distinct channel**（另有 4 类工厂不计入这 21 集）+ 信号/透传/控制三类 + 命名陷阱词典。配 `tests/projection_schema_test.rs` 强制字段对账。（review D14：原「21 工厂」是把「21 channel」误当工厂数。）

### 4.3 byte-parity 保证

- **保证**：native 逐条 broadcast 即时 send vs ffi 攒批 flush，**交付内容 byte-identical**（仅传输管道不同）。
- **证据**：`tests/f5_egress_parity.rs` — 两 sink 喂同一份预收集 source，断言 batch roundtrip vs broadcast 直传**字节一致**。CI 证伪保护：break flush→FFI 空、break decode→错位，均 FAIL（HX-C011 可证伪）。
- **已知边界**（诚实声明）：f5 测的是 **egress 管道 parity，非引擎 parity**（引擎双端一致靠共享 `run_engine_loop` + 同一 `ImModule` by-construction 论证，非测试断言）；**lagged/慢消费下不覆盖**（见 §7-G3）。

---

## 5. 边界声明

- **本仓交付止于 C ABI**：`helix.hpp` + cdylib/staticlib + ffigen 可消费的 C 头（`helix.h`，见 §8 F 阶段）。
- **Dart/Flutter 工程在仓外**：`NativeCallable` 包装、`ffigen` 配置、widget 树、isolate 管理由 Dart 侧消费本契约实现，**不进本仓 CI 必跑矩阵、不进交付验收**。
- **ffigen 消费 helix.hpp/helix.h**：Dart 侧 `ffigen` 走 libclang 解析 C 头生成绑定。本仓负责保证 C 头干净可消费（见 §7-G8 cbindgen 双配置）。
- **`usize → uintptr_t` 标注**：Dart 侧绑定须用 `ffi.Size`（Dart 2.17+），**禁 `Uint64`**，否则 32-bit ABI（armv7 老机）错位（§7-G14）。

---

## 6. 性能不变量（可证伪压测清单）

> 落 `crates/helix-driver-ffi/docs/PERF-INVARIANTS.md`，纳入 §4.7 drift-review lens。所有断言用**计数/结构/单调性**，**禁墙钟下界断言**（HX-C011）。

| # | 不变量 | 校验方式（可证伪 + mutation） | 当前状态 |
|---|---|---|---|
| **PI-1** | 命令热路径 payload 仅 1 次必要拷贝（ACL-3）；name 命中闭集时 0 分配 | alloc 计数器：命中 name 的 `helix_command` 净堆分配 == 1（payload）。**破坏验证**：去掉 name 闭集映射 → 分配==2 → fail | payload ✅ / name **待优化**（现 `Cow::Owned` 每次 `to_owned`，见 §7-G6）|
| **PI-2** | egress 全程零额外字节拷贝（`Bytes` Arc clone，非深拷贝） | 类型断言 `AppCommand.payload`/`DomainEventBytes` 是 `Bytes`；grep 链路无 `.to_vec()`/`copy_from_slice`（除 ACL-3 入口） | ✅ 已达成 |
| **PI-3** | 一 step 的 N 条 Emit（同一 step 内）**恰好 1 次**过 FFI 边界（HX-C007） | 压测断言 `on_domain_events_batch` 调用次数 == **非空 step 数**（非事件数）。**破坏验证**：flush 改 emit 内逐条回调 → 次数==事件数 → fail（**最该先写的守门测试**）| ❌ **未达成可证伪**：机制设计达成但**无守门测试**，f5 结构上测不到逐条破坏（emit 全收再单次 flush，断字节并集与回调次数无关）→ 须 Phase 5 新写 multi-step 回调计数测试（见 §7-G7）|
| **PI-4** | encode 缓冲稳态零分配（scratch 复用；**依赖 §3.3 借出契约**，非 ownership-transfer）| alloc 计数：稳态 step 编码路径净分配 == 0；scratch capacity 不增长。**破坏验证**：改回每 step 新 Vec → 稳态分配>0 → fail | ❌ **待实现**（现 `batch.rs:33` 每 step 新 `Vec`）。**借出模型下可达成**（buffer 回调内消费完即可复用 scratch）；改 `encode_batch_into(&mut scratch)` 即可。R5「永远 fail」仅在 ownership-transfer 下成立，已不适用 |
| **PI-5** | 批 buffer 容量单次预估，零 realloc | 代码断言 `Vec::with_capacity(精确值)`；编码中无再分配 | ✅ `batch.rs:33` 已预估，勿退化 |
| **PI-6** | 命令背压：channel 满返 `FFI_BUSY`，零静默丢弃，零无限缓冲 | 压测 `FFI_OK + FFI_BUSY == 发送总数`；cap 有界(256)。**破坏验证**：try_send 改 send / cap 改 ∞ → BUSY 永不出现 → fail | 机制 ✅ `api.rs:109,377` / **守门测试待 Phase 5**（与 PI-3 对齐）|
| **PI-7** | core step 主循环不等 I/O（Persist/Http 入队即返回） | native stress_engine 守 Block 死锁安全；ffi 共用核（五不变量②）| ✅ |
| **PI-8** | 阻塞 I/O 全 `spawn_blocking` 卸载，不占 core 线程 | grep：`storage.rs` 所有 rusqlite 调用包 `spawn_blocking`（HX-C006 闸门）| ✅ |
| **PI-9** | blocking pool 上限显式封顶（防 DNS 风暴线程暴涨）| 代码断言 `Builder::new_current_thread().max_blocking_threads(N)` 显式设置 | ❌ **待加**（现默认 512，见 §7-G11）|
| **PI-10** | p99 单 step CPU 时间有预算上限（防单线程被慢 step 卡死）| 压测采 step **CPU 时间**分布（非墙钟），p99 < 预算作回归基线（非硬下界）| 待建基线 |
| **PI-11** | 生命周期内存稳态：create/destroy 多轮 RSS 斜率≈0，Arc/Box 计数归零 | leaks/valgrind + Arc strong_count 断言（用多轮采样回归斜率，非绝对阈值）。**破坏验证**：注释 `helix_destroy` 的 `Box::from_raw` → leaks 报泄漏 → fail | 部分（lifecycle drain 已有，泄漏检查待补）|

**单线程核不变量补充**（reconciled 五不变量，ffi 单线程下逐条成立，标「无丢事件前提」）：① 回灌必达（`reply_tx` unbounded）② 主循环不等 I/O ③ 同 key 写序（**靠 Persist `BoundedSpawner` N=1 单消费者保证，与线程模型无关**——current_thread 下并发 worker task 仍交错 await，native multi-thread 同机制；**禁因「单线程」放宽 N=1，否则写序 bug**。review P1-1 修正：原「单线程下冗余」表述错误）④ 回灌优先（`biased select reply_rx` 在前）⑤ graceful drain（`HTTP_DRAIN_LIMIT=3s` + ffi 额外 `shutdown_timeout(ZERO)` detach getaddrinfo）。

---

## 7. Gap 台账 + 决策

> 引用 reconciled 台账（三份分析对抗式核对）。P0 = 防 UB / 确定性断裂；P1 = parity 裂缝 / 性能 / ffigen；P2 = 记一笔 / 体验优化。

### 7.1 三大论断裁决（入 spec 基石）

| 论断 | 裁决 | 附带条件 |
|---|---|---|
| **C ABI 面不随 UC 增长** | ✅ **成立**（三方独立确认，证伪失败）| 必写 §1.4 三条已知边界，防「不增长=够用」偷换 |
| **helix_query=A 异步足够** | ✅ **成立**（两方确认：异步管道+loading skeleton 覆盖冷启动首帧；同步反而阻塞 UI isolate=反模式，且必泄漏 IM schema 违 HX-C001）| 业务查询返回前须清零 out 参数（G-10）；capabilities/version 仍通过该 ABI 返回 |
| **ffi 单线程语义等价** | ✅ **成立（仅无丢事件前提）**；lagged 下**证伪成功**=裂缝（G-3）| 五不变量逐条成立；**③ 写序靠 N=1 单消费者非线程**（原「③冗余」措辞错已修，P1-1）；⑤更强；lagged deadline 锚 M2 |

### 7.2 Gap 决策表

| # | gap | 真实性 | 优先级 | 决策 |
|---|---|---|---|---|
| **G-1** | ~~Dart listener 异步下 buffer UAF~~ **(review 证伪)** | **驳回→降级** | ~~P0~~ **文档** | **维持借出契约（零 Rust 改动）**：C++ shim 回调线程 memcpy 稳定化吸收 Dart 异步（demo 已 done）。**删 `helix_free_batch`**（破 7 符号冻结）。§3.3 |
| **G-2** | `now_ms` host 写清是 C++ shim（非 Dart）| 确认→降级 | ~~P0~~ **文档** | **保留 vtable.now_ms 字段**，由 C++ shim 同步填（现状 clock.rs:38 已优先调，链路已对）；**禁移除字段**（违 GOAL §1.2）；测试用 mock now_ms 跑回放。§3.2 |
| **G-3** | lagged/慢消费下 native(resync_needed) vs ffi(无信号) parity 静默失效（**native 演进对齐项**）| 确认 | **P1** | **判据=M2 接 `EventReceiver→resync_needed` 的同一 commit**。对等信号**裁定走 `im_request_resync` 经 helix_command 回灌**（不破 ABI）；**否决「回调返回码反馈 host 消费位」**（改 `on_domain_events_batch` 签名 = 破 §1.1 七符号冻结）。**禁在 Rust FfiEventSink 内造假有界 ring 模拟 broadcast lagged**。实现延 M2，本轮留契约占位（review D23）|
| **G-4** | ~~scratch 复用 vs ownership-transfer 互斥~~ **(随 G-1 消解)** | 驳回 | — | 借出模型下编码 buffer 始终 Rust own、回调返回即可复用 → **互斥前提不成立**。PI-4 scratch 复用照常做（§3.3 / PI-4）|
| **G-5** | `panic="abort"` 仅 release；dev/test/staticlib-dev-link unwind 无 catch_unwind = 隐藏 UB | 确认 | **P1** | spec 声明「**FFI 边界安全仅 release(panic=abort) 保证；dev/staticlib-dev-link 仅供 rlib Rust 集成测试，非 FFI 安全构建**」。cpp-smoke/ffigen-smoke 用 release。禁两者混说（HX-C006）|
| **G-6** | name 热路径每命令 `Cow::Owned` 堆分配（PI-1 当前为假）| 确认 | **P1** | name 闭集→`&'static str` 映射（driver 边界一行 match + 静态表，`Cow::Borrowed` 命中免分配，不动 core）|
| **G-7** | HX-C007 批不变量(PI-3) + 背压(PI-6) 无守门测试 | 确认 | **P1** | **新写**两个证伪测试（最该先写）：PI-3 multi-step 回调**计数**测试（≠ f5 的字节 parity，二者正交，f5 测不到逐条破坏）；PI-6 `FFI_OK+FFI_BUSY==N` 测试 |
| **G-8** | `helix.hpp` 含 `<ostream>/<new>` C++ 头污染 ffigen | 存疑(单源) | **P1** | **cbindgen 双配置**：C++ smoke 用 `helix.hpp`(`language="C++"`)，Dart ffigen 用单独 `language="C"` 的 `helix.h`（无污染）。需 ffigen 实测确认 |
| **G-9** | `api.rs:102 trim_end_matches("/api/cses")` 业务 URL 泄漏进 driver（违 HX-C001）| 确认(单源证据硬) | **P2** | 记一笔，实现延后：下沉 helix-im `ImConfig` 或 env 注入(`HELIX_DEFAULT_API_BASE`)。两端一致泄漏故 parity 成立但方向错。M2 前低优 |
| **G-10** | `helix_query` stub 返回前不清零 `*out_ptr/*out_len` | 确认 | **P2** | 一行修：返回 `FFI_UNIMPLEMENTED` 前先 `*out_ptr=null; *out_len=0` |
| **G-11** | `max_blocking_threads` 未封顶（默认 512）| 确认 | **P1**（FFI 语境，review D16 升级）| 一行硬化 `Builder::...max_blocking_threads(32)`（≈ READ_POOL_SIZE+max_http_inflight+余量）。移动端弱网 DNS 风暴比 native 高发，故 FFI 注脚升 P1 |
| **G-12** | `FFI_INVALID` 信息量过低，缺 `helix_last_error` | 存疑 | **P2** | spec 记选项：thread-local 最后错误串（C 侧只读不释放）。非阻塞，体验优化 |
| **G-13** | F5 是 egress parity 非引擎 parity | 确认(已知边界) | **P2** | **记 backlog**：补两端各自 `shell.step` 同剧本引擎 parity 测试。by-construction 论证当前充分，M2 前不阻塞 |
| **G-14** | `uintptr_t len`(usize) 映射，armv7 须用 `ffi.Size` | 确认 | **P2** | 契约标注（§5）|
| **G-15** | tick channel cap=256，大 payload(附件) 撞容量 | 确认(吞吐非 ABI) | **P2** | **记 backlog**：ABI 面不破，附件 UC 落地前 YAGNI |
| **G-16** | HX-C002 须补「ffi=native-target 单线程，Send 不放松」| 确认(防误传) | **P2** | harness 更新：补「ffi 是 native-target 单线程**运行时**(非编译期 Send 放松)；放松 Send 仅 wasm32(已归档)；host engine bound 禁改 MaybeSend」|
| **G-17** | `storage.rs` cursor 特化 SQL（`channel_event_cursor` 等表名 `Cow::Borrowed` 快路径）= 第二处疑似 schema 泄漏 | 存疑 | **P2** | 记一笔：边界在「**性能特化白名单 vs 业务语义解释**」——表名/列名是 IM 经 spec 传入的 `&'static str`，driver 只做字符串特化不解释含义。待澄清，不阻塞本轮（review D19）|

### 7.3 关键 Gap 的「多模块 = YAGNI」立场

- **多模块公平性 / 跨模块调度**：native CLAUDE.md「刀3 缺口」——IM 单模块无可观察行为差，**第二真实模块出现前 YAGNI**，不预建。

---

## 8. 实现计划（分阶段，每阶段独立可验，commit 节点）

> **铁律：cargo 验证仅放最末 §9 阶段，中途禁 cargo**（HX-C 项目 Cargo 纪律：靠类型签名 + rust-analyzer 自审）。下方标注每阶段「纯文档/契约(零代码)」vs「真改码」。

### Phase 0 — SPEC 落地 + 模块索引挂接（纯文档，零代码）
- 本 SPEC MD 落 `crates/helix-driver-ffi/docs/SPEC-full-takeover.md`。
- 模块 `crates/helix-driver-ffi/CLAUDE.md` + `AGENTS.md`（镜像）挂索引。
- 新建 `crates/helix-driver-ffi/docs/PERF-INVARIANTS.md`（§6 表）。
- **commit**：`docs(ffi): 全量接管 SPEC + PERF-INVARIANTS + 模块索引`

### Phase 1 — P0 契约澄清（**纯文档/注释，零 ABI 改码**；review 修正后大幅缩水）
- **G-1**：`on_domain_events_batch` **维持借出契约**——`event_sink.rs:72-79` 不动；helix.hpp/vtable.rs 注释写清「ptr 仅回调期间有效，shim 回调线程 memcpy 稳定化」。**不加 `helix_free_batch`**（保 7 符号冻结）。
- **G-2**：`now_ms` **保留 vtable 字段**；clock.rs:38 链路已对（优先调 vtable），仅注释写清 host=C++ shim（非 Dart）+ 测试用 mock now_ms。**不改 ABI、不移字段。**
- **commit**：`docs(ffi): P0 契约澄清 now_ms host=shim + 投影批借出契约（删 ownership-transfer 误判 G-1/G-2/G-4）`

### Phase 2 — P1 性能硬化（真改码，局限 ffi）
- **G-6**：name 闭集→`&'static str` 映射（`api.rs` 边界 match + 静态表，`Cow::Borrowed` 命中免分配）。
- **G-11**：`Builder::new_current_thread().max_blocking_threads(32)`（`pump.rs:68`）。
- **PI-4**：scratch 编码缓冲复用（`encode_batch_into(events, &mut Vec<u8>)` 重构，与 Phase 1 所有权模型协调）。
- **commit**：`perf(ffi): name 闭集免分配 + blocking pool 封顶 + scratch 复用（G-6/G-11/PI-4）`

### Phase 3 — P1 panic profile 声明 + ffigen 双配置（部分纯文档 + 部分改码）
- **G-5**：Cargo.toml / 模块文档声明「FFI 安全仅 release」（纯文档 + 必要时 cpp-smoke 切 release profile）。
- **G-8**：cbindgen 双配置——新增 `language="C"` 生成 `helix.h` 供 ffigen；保留 `helix.hpp`(C++) 供 cpp-smoke。
- **commit**：`build(ffi): cbindgen 双配置(C头供ffigen) + panic profile 安全声明（G-5/G-8）`

### Phase 4 — P2 防御性收口（真改码，一行级）
- **G-10**：`helix_query` stub 返回前清零 out 参数。
- **G-9**：记一笔（URL 泄漏下沉 ImConfig/env，实现可延 M2，本阶段仅文档标注 TODO + 不动逻辑）。
- **G-12**：`helix_last_error` 选项**记 spec**（非本轮实现）。
- **G-14**：`ffi.Size` 标注（纯文档）。
- **commit**：`fix(ffi): helix_query stub out 清零 + 边界标注收口（G-10/G-14）`

### Phase 5 — P1 守门测试（真改码，测试）
- **G-7 / PI-3**：批不变量守门测试（`on_domain_events_batch` 调用次数 == 非空 step 数，破坏=逐条则 fail）。
- **G-7 / PI-6**：背压守门测试（`FFI_OK + FFI_BUSY == N`，破坏=无限缓冲则 fail）。
- **G-3**：lagged 对等 resync 信号**契约占位**（测试桩 + 文档 deadline 锚 M2，**不实现假有界 ring**）。
- **PI-11**：lifecycle 泄漏检查补全（Arc/Box 计数断言）。
- **commit**：`test(ffi): HX-C007 批不变量 + 背压 + lifecycle 泄漏 守门测试（G-7/PI-3/PI-6/PI-11）`

### Phase 6 — harness 更新（纯文档，零代码）
- HX-C002 补「ffi=native-target 单线程运行时，Send 不放松」（G-16）。
- HX-C006 补 G-1 **buffer 借出契约（ptr 仅回调期间有效，shim 回调线程 memcpy 稳定化吸收 Dart 异步；不新增 free 符号）** + G-5 panic profile 声明。
- 同步 `docs/harness/README.md` 索引 + 根 CLAUDE.md/AGENTS.md HX-C 表。
- **commit**：`docs(harness): HX-C002/C006 补 ffi 单线程Send/buffer所有权/panic profile（G-16）`

---

## 9. 验证计划（最末执行，**仅此阶段允许 cargo**）

> 前 8 阶段全程禁 cargo；所有编译/测试集中在此一次性跑（autonomous 模式自跑回报，单任务模式输出命令让用户决定）。

```bash
# ① 双端编译验收（零运行时证明 + 全 crate check）
cargo check -p helix-driver-ffi
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'   # 必须空

# ② clippy 护栏
cargo clippy -p helix-driver-ffi -- -D warnings

# ③ ffi 单元/集成测试（含 egress parity + 新增守门测试）
cargo test -p helix-driver-ffi                # 含 f5_egress_parity / PI-3 批不变量 / PI-6 背压 / PI-11 泄漏

# ④ 回归（确保 native/host 未被 ffi 改动破坏）
cargo test -p helix-driver-native -p helix-driver-host

# ⑤ C++/ffigen 消费 smoke（release profile，FFI 安全前提）
scripts/cpp-smoke.sh                          # 含 helix.hpp 链接 + 30s ctest（shutdown_timeout(ZERO) 修复后通过）
```

**验收判据**：
- ① `cargo check` 全过 + `cargo tree` grep 空（HX-C001 零运行时）。
- ② clippy `-D warnings` 零告警。
- ③ ffi 全测试 PASS，**含 `f5_egress_parity`（byte-parity）+ PI-3/PI-6 守门测试破坏即 fail 已验证**。
- ④ native + host 回归全过（无副作用）。
- ⑤ `scripts/cpp-smoke.sh` PASS（C ABI 链接 + 生命周期无 ctest TIMEOUT/泄漏）。
- **终态行**（写 SESSION §0）：`✅ DONE driver-ffi 全量接管 SPEC 落地 + P0/P1 硬化 + 守门测试 @<ts> | commit <起>..<止> | 7阶段 | helix/<分支>`
