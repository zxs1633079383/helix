# Helix 完整 Roadmap

> 本文回答“为什么按这个顺序把一份业务与交互语义交付到 PC、Mobile、Web”。
> 天级任务、当前 pin 和阻塞见 [`ROADMAP.md`](./ROADMAP.md)；目标常量与 TSCR 见
> [`GOAL.md`](./GOAL.md)。
>
> 2026-07-27 重构：删除已被 ADR-014/015 覆盖的“Web archived”和旧 D+A/ViewModel
> 路线，不再把已完成的 FFI F4、Web A1-A5 或 InteractionFrame V2 写成未来任务。

---

## 1. 产品终局

```text
                        ┌─────────────────────────────┐
UiEvent / Observation ─▶ helix-im domain + surfaces  │
                        │ action / effect / persist   │
Authority / PortReply ─▶ InteractionFrame V2         │
                        └──────────────┬──────────────┘
                                       │ deterministic bytes
                 ┌─────────────────────┼─────────────────────┐
                 ▼                     ▼                     ▼
           PC Tauri/Angular      Mobile FFI/UI         Web WASM/UI
           mechanical apply     mechanical apply      mechanical apply
           platform I/O         platform I/O          platform I/O
```

最终不是“三端调用同一套 API”这么弱，而是：

- 业务动作选择、权限、状态转换、窗口、未读、temporaryId 对账和 fallback 只在
  `helix-im`；
- 三端只采集用户意图、执行 Effect、机械 apply Frame 和渲染平台组件；
- durable 结果都经过 authority + PersistOk；
- 任一 Gate 的三端结果可由同一 correlation 和 evidence window 复查；
- 用 TSCR 衡量 active Gate 的三端同核闭环，不再用“代码完成度”或单端百分比。

---

## 2. 两条正交扩展轴

```text
业务轴： IM ── Presence ── Meeting ── Notify ...
平台轴： Native ── FFI ── Web ── future platform ...

共享交叉点：helix-core ports + deterministic execution
业务语义层：每个 Module 自己的 domain + InteractionProtocol
平台差异层：I/O capability + renderer registry
```

复杂度目标从 `业务数 × 平台数` 降到 `业务数 + 平台数`。成立条件不是 crate 可以编译，
而是平台没有复制业务 reducer。第二业务模块和 TSCR 都是这个主张的反证工具：

- 第二模块证明 core 不是 IM 专用框架；
- TSCR 证明三端没有只在静态合同上“看起来共用”。

---

## 3. 不可交换的实施顺序

### 3.1 先确定性地基，再接真实 I/O

如果 core 先依赖 tokio、SQLite 或 WebSocket，FFI/Web 会被 native 线程模型锁死。
因此先完成 Tick/Effect/port/Module host、时间/ID 注入、回放和乱序 gate，再让 driver
实现 I/O。

### 3.2 先权威事实与 Persist barrier，再做 UI 语义

HTTP accepted 或 WS frame 不是业务完成。先建立 authority -> PersistOk -> projection 的
顺序，才能让 InteractionFrame 的 revision 和终态可信。否则统一 UI 只会统一错误。

### 3.3 先共享 V2 writer/reader，再逐 Gate 迁移

V1/V2 双写、客户端 normalize 和 raw reducer 会制造无法观察的 fallback。必须先建：

- V2-only codec；
- atomic SurfaceRuntime；
- epoch/revision/transaction；
- ActionBinding 与 FrameApplied；
- source pin/fixture/evidence contract。

之后每个 Gate 才能用同一模板纵切。

### 3.4 先一个 Gate 三端闭环，再扩全量

PC 26 Gate、Mobile 27 contract Gate 或 Web smoke 各自变绿，仍可能完全没有三端同窗
闭环。P0 先让 G01 产生首个 ThreePlatformGreen，再复制方法到其他 active Gate。

### 3.5 最后物理删除 legacy

静态 hard-cut 是准入，不是删除授权。只有固定 candidate 的 required fail/skip 为 0、
V1/raw counters 为 0，并且删除后回归再次通过，才删除旧 writer/reader/fixture/docs。

---

## 4. 阶段全景

| Phase | 目标 | 状态 | 退出证据 |
|---|---|---|---|
| F0 | runtime-agnostic core 骨架 | 完成 | core 零 runtime、Module/ports、deterministic replay |
| F1 | IM 收发/同步/持久化地基 | 完成 | 真 Go、乱序/cursor、echo、Persist barrier |
| F2 | native + shared host | 完成 | SQLite/HTTP/WS/engine loop、Tauri host |
| F3 | FFI/C++ 产品接线 | 完成到平台接线 | shared loop、batch event、Rust 自持 I/O、ABI |
| F4 | Web Rust 平台层 | 完成 | final WASM、Browser Fetch/session/WS、真 Go smoke |
| F5 | InteractionProtocol V2 共享基线 | 完成 | V2 codec/writer/Surface/Action/ack、canonical fixture |
| F6 | G01-G16c PC/Mobile 纵切 | 主体完成，证据收口中 | per-Gate review、当前 pin 总控、legacy 删除后回归 |
| F7 | Web 产品 renderer + 首个三端 Gate | 未完成 | G01 ThreePlatformGreen，TSCR > 0 |
| F8 | 43 active Gate 三端扩展 | 未完成 | current epoch 全部 ThreePlatformGreen |
| F9 | 第二业务模块 | 未完成 | 新 Module/core 零改、至少一条三端闭环 |

---

## 5. 已完成地基

### F0：确定性内核

产物：

- `Module` host；
- Tick/Effect 执行模型；
- FrameSender、Storage、HTTP、Clock、Timer、EventSink、IdSource 等 ports；
- owned 状态、单线程可运行、时间/ID 注入；
- 回放、乱序、cursor 单调与 HX-C 护栏。

保持的红线：

- core 不认识 channel/message/UI；
- core 不执行 I/O；
- feature additive；
- 加端和加模块不修改 core。

### F1：IM authority 与持久化

产物：

- hello、increment、sync、pong gap、unknown channel hydration；
- 发送 optimistic/Persist/HTTP/WS echo/reconcile；
- phantom、readBits、cursor、离线恢复；
- local-first/remote fallback 与明确 coverage；
- durable projection 等待 matching PersistOk。

### F2/F3：native/FFI 共用 host

产物：

- shared SQLite schema/StorageOp；
- shared HTTP/WS 与 header registry；
- shared `run_engine_loop`；
- native/FFI 薄壳；
- FFI opaque handle、批量 event、C++-host 直连。

这一步已经淘汰旧“Dart 喂帧、Rust 只做库”的路线。Mobile UI 仍需独立真机证据，
但不再需要另一套业务 engine。

### F4：Web 平台层

产物：

- `helix-driver-web`；
- Rust Browser Fetch；
- bounded page-session store/Disabled；
- ExternalWsBridge、browser timer、bounded event batch；
- final WASM 与 host-runtime leak gate；
- 浏览器 query 鉴权、hello、send/echo、断连恢复。

边界：该 crate 明确不做 frontend rendering。Web 平台 I/O 完成不等于 Web 产品 Gate
完成。

---

## 6. InteractionProtocol V2 主线

### F5：共享协议基线

已经建立：

- `InteractionFrameV2` 的 runtime、mutations、directives；
- typed Surface/VM、snapshot/patch；
- `surfaceEpoch`、`revision/baseRevision`、transaction；
- opaque `ActionBinding` 与独立 ActionInvocation schema；
- whole-frame staging/commit；
- FrameApplied、ack/resync；
- canonical fixture/provenance/source pin；
- safe correlation 与 evidence contract。

协议边界：

- `helix-im` 决定语义；
- driver 传输批次；
- SurfaceRuntime 只验证和原子提交；
- renderer registry 只映射具体组件；
- raw event 不写业务 VM。

### F6：PC/Mobile 逐 Gate 纵切

第一批范围为 GS01-GS04 + G01-G16c，共 27 Gate。当前进展：

- Helix producer、canonical fixture、PC/Mobile typed consumer 已覆盖主要范围；
- G01-G16 的逐 Gate review 主体已完成；
- G16b/G16c 代码与 fixture 已进入 main，但总控任务状态和最终 program report 尚需收口；
- GS01-GS04、legacy 删除选择与删除后全量回归仍是开放项；
- PC/Mobile 都已 pin `98c522a`，但 current-epoch runtime evidence 仍需同窗重建。

此阶段的退出标准不是“27 个子项目都提交”，而是：

```text
single candidate
+ current Matrix/hash
+ PC aggregate real evidence
+ Mobile aggregate real evidence
+ required fail=0 / skip=0
+ V1/raw=0
+ approved deletion
+ post-delete regression
```

---

## 7. 三端闭环主线

### F7：Web renderer 与首个 ThreePlatformGreen

最小纵切选择 G01，因为它同时覆盖：

- composer ActionBinding；
- optimistic Sending；
- Persist barrier；
- HTTP create；
- WS echo/reconcile；
- timeline/dialog 原子 Frame；
- actor/observer；
- ack/resync；
- 三端最终 UI。

Web 必须新增的是平台实现，不是业务逻辑：

- V2 codec/Surface slot 或复用生成合同；
- static renderer registry；
- action invocation；
- FrameApplied/ack/resync；
- product DOM 与 evidence hooks。

禁止：

- 在 JS 中解析 raw IM event；
- 重做 temporaryId 对账；
- 由 Web 自选业务 command；
- 用 browser smoke 代替产品 DOM。

F7 退出时 TSCR 从 `0/43` 变为至少 `1/43`。

### F8：扩到全部 active Gate

每个 Gate 使用同一流水线：

```text
Matrix/Spec
→ failing contract/replay
→ Helix command/observation + authority + Persist
→ canonical Frame V2
→ PC/Mobile/Web mechanical consumer
→ current-pinned mock + real + dual + persistence + UI
→ ThreePlatformGreen
```

实施批次：

1. GS01-GS04 与 G01-G16c 当前批次收口；
2. G03 media；
3. G17-G21 频道/成员/公告/presence；
4. G25-G27 书签、待办/通知、公司大群与推送；
5. G22-G24 只有在新 epoch 明确激活后进入。

Gate 拆分、合并或 lifecycle 变化必须升级 Matrix epoch；TSCR 必须带新分母和 hash。

---

## 8. 媒体能力支线

ADR-016 已确定 Rust 直连 Java：

```text
Helix command
→ Java prepare
→ driver PUT object
→ Java complete/verify
→ stable capability URL
→ Go posts/create
→ WS/authority/Persist/Frame
```

当前 v1 边界：

- PC/FFI shared host 支持文件上传；
- 64 MiB 单 PUT 是当前实现边界；
- Web 有双地址配置槽，但 File/Blob upload 与 CORS 未完成；
- G03 必须保持 active/declared，直到 current-pinned 真实 object readback 和三端产品证据完成。

不能再把旧用户 waiver 当永久产品状态，也不能用 Go posts/create 200 替代对象完成与回读。

---

## 9. 第二业务模块

当 IM 的首个 ThreePlatformGreen 稳定后，引入一个形状不同的真实模块：

- Presence：高频、短生命周期、弱持久；
- Meeting：多阶段状态、timer、成员权限；
- Audit：HTTP + Storage，可能没有 WS。

选择标准是能反证 IM 偶然性，而不是最容易演示。完成条件：

- 新模块只实现自己的 domain、Module、ports 使用和 InteractionProtocol；
- `helix-core` 一行不改；
- 三端只新增 renderer/capability adapter；
- 至少一个 active Gate 达到 ThreePlatformGreen；
- 多模块共享 engine/storage 的性能与公平性有数字证据。

---

## 10. 性能与可靠性预算

| 边界 | 约束 |
|---|---|
| core step | 单线程、确定性、无 ambient I/O |
| 热路径 | O(1) 查找/推进，禁无界 scan 与 read-modify-write |
| Storage | 单写者、batch、PK/索引命中、事务原子 |
| FFI/Web | Frame/step 批量传输，队列有界并报告 gap/drop |
| Surface | revision O(1) 判定；大列表使用有界 window/patch |
| Action | opaque capability O(1) lookup，过期 fail-closed |
| Evidence | correlation 有界、脱敏，不记录 token/cookie/正文/完整 VM |

性能主张必须来自 benchmark/stress artifact；没有数字时只写设计目标。

---

## 11. 发布与停止条件

### 发布条件

- source pin、Matrix hash、fixture、generated contract 一致；
- scoped required fail=0、skip=0；
- authority/Persist/Frame/UI/trace 同窗；
- V1 producer/consumer/raw fallback 为 0；
- 变更影响的三端 gate 与 ABI/build 通过；
- TSCR 只由 machine reducer 派生。

### 停止条件

- 目标要求违反 GOAL 不变量；
- Matrix/Spec 与代码的业务语义冲突且没有 ADR；
- current candidate pin/hash 不一致；
- 缺少真实 connector/credential/设备且无法继续收集对应证据；
- 同一失败在授权范围内连续三次仍无进展；
- 为绿灯需要弱化断言、制造数据、恢复 legacy 或越过 owner 边界。

停止时输出 `partial` 或 `blocked`，列明已完成证据、第一阻塞面和恢复动作。

---

## 12. 战略完成

Helix 的战略完成态是：

1. 所有 current-epoch active Gate 的 TSCR 为 100%；
2. 三端没有业务 reducer、命令选择或权威字段补算；
3. 第二业务模块在 core 零改条件下完成三端闭环；
4. 新平台接入只实现 ports、bridge 和 renderer；
5. 性能、可靠性和证据边界由 HX-C/CI/machine reducer 持续守住。

在此之前可以准确声明平台层完成、合同完成、Gate review-green 或某端 real-pass，
但不能声明“Helix 三端目标完成”。
