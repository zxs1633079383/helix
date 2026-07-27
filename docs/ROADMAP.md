# Helix 当前 Roadmap

> 本文是当前活账：只记录已经由代码/证据确认的状态、下一阶段、阻塞和停止条件。
> 战略原因见 [`ROADMAP-full.md`](./ROADMAP-full.md)，目标与唯一北极星指标见
> [`GOAL.md`](./GOAL.md)。
>
> Snapshot：2026-07-27。动态状态必须绑定 revision、Matrix epoch/hash 与 evidence window，
> 不从旧报告继承。

---

## 1. 当前结论

Helix 已经从“native/FFI 共用业务状态机”推进到“InteractionFrame V2 生产合同和 PC/Mobile
消费骨架大范围落地”。当前最大缺口不再是 core 或 driver 设计，而是：

1. 把三端消费 pin、Matrix、fixture 与运行证据收敛到同一个 candidate；
2. 关闭 G16b/G16c、GS01-GS04 和删除后回归等总控尾项；
3. 建立 Web 产品 renderer，使 Web 不再只有平台 smoke；
4. 用同一证据窗口计算 TSCR，而不是拼接 PC、Mobile、Web 的历史局部 Green；
5. 继续实现 G17-G27 active Gate，并完成 G03 当前媒体合同的真实发布证据。

当前 TSCR：`0/43 = 0%@epoch11+caadafd2...`。原因见 [`GOAL.md`](./GOAL.md#5-唯一北极星指标)。

---

## 2. 代码与合同快照

| 对象 | 当前事实 | 证据边界 |
|---|---|---|
| Helix 仓 | `main@3ddce5e9`，`origin/main@dd598c9c`；HEAD ahead 1 | 最新提交是 Apifox 文档证据，不自动改变消费 pin |
| PC 仓 | `feat/helix-reactor-im@af0f46ff9`，ahead 3 | 正在继续 message-v4 UI 工作 |
| Mobile 仓 | `feat/reactof-helix-v2@a358094` | 存在未跟踪 bundle，不能清理或覆盖 |
| PC Helix pin | `98c522a3621cc599c134cb78b9e4656668eb67bb` | `src-tauri/Cargo.toml` 四个 Helix crate 一致 |
| Mobile Helix pin | `98c522a3621cc599c134cb78b9e4656668eb67bb` | `contract/helix-source.lock.json` |
| Strict Matrix | epoch 11，raw SHA `caadafd2947c...` | 46 Gate：43 active、3 excluded；active 当前仍标 `declared` |
| InteractionFrame | V2 writer/codec/typed Surface/Action/ack 已落代码 | ActionInvocation 仍是独立 schema v1 |
| canonical fixture | 22 个 provenance 目录、104 个 Frame V2 文件 | 证明 producer/合同，不单独证明 runtime |

工作树保护：

- Helix 有用户现存的消息审批文档修改；
- PC ahead 3；
- Mobile 有未跟踪 bundle/`.DS_Store`。

本 Roadmap 的执行不得 reset、clean、覆盖或把这些改动混入无关提交。

---

## 3. 分面状态

### 3.1 内核与平台地基

| 能力 | 状态 | 当前事实 |
|---|---|---|
| runtime-agnostic core | 完成 | core ports、Module host、sans-IO、确定性与零 runtime 护栏已建立 |
| shared host | 完成 | native/FFI 共用 SQLite、HTTP/WS、engine loop、header registry |
| FFI/C++ host | 完成到产品接线 | command、批量 event、Rust 自持 HTTP/WS 已接；真机 UI 是独立证据面 |
| Web driver | 平台层完成 | Browser Fetch、session store、WS bridge、timer、final WASM 与真 Go smoke 已完成 |
| Web product UI | 未完成 | driver 明确不含 frontend rendering；无全 Gate renderer/action/apply/ack |
| 第二业务模块 | 未开始 | core 通用性仍主要由接口和测试证明，尚无第二个异形模块裁决 |

### 3.2 InteractionProtocol

| 能力 | 状态 | 证据 |
|---|---|---|
| V2-only shared writer/reader | 完成 | Spec 018 Phase 2；Helix/PC codec/runtime validators |
| typed Surface/VM | 已覆盖 G01-G16c 主要 Surface | `interaction_frame.rs`、`projector/`、canonical fixtures |
| opaque ActionBinding | 已落地 | actor/scope/epoch/revision 绑定与 action ledger |
| Persist barrier | 已落地到目标 Gate | durable Frame 等待 matching PersistOk |
| V1 生产/消费硬切 | 目标范围已有静态/局部证据 | 最终删除仍受总控和删除后回归约束 |
| 三端统一产品 renderer | 未完成 | PC/Mobile 有消费实现；Web 产品 renderer 缺失 |

### 3.3 Gate 进度

当前必须同时保留三套分母：

- 全产品合同：46 Gate，43 active；
- InteractionFrame V2 第一批：GS01-GS04 + G01-G16c，共 27 Gate；
- Spec 018 V2 物理硬切：25 Gate，明确排除 G03/G03a。

已确认：

- G01-G16 的逐 Gate Helix producer、PC/Mobile 合同和历史 review evidence 已大范围闭合；
- G16b/G16c 的 Helix 生产代码、fixture 与三仓同步提交已经进入 main 闭环；
- `specs/020` 的 T022-T032 尚未全部勾选，GS01-GS04、program report、V1 删除选择和
  删除后全量回归仍是开放项；
- 不得把代码已合入或子 Gate review-green 写成总控 `CLOSED`；
- G17-G21、G25-G27 仍是 active 但未进入本批次纵向实现；
- G22/G23/G24 为 excluded，不进入 TSCR 分母；
- G03 是 active/declared，媒体 Rust/Java 边界按 ADR-016 推进，不能再用旧 waiver
  永久隐藏。

---

## 4. 三端消费现状

### PC

已具备：

- Tauri `helix_im` host；
- V2-only codec、SurfaceRuntime、adapter/facade、FrameApplied；
- message-v4 typed components、opaque action、Mock/single/dual/report harness；
- 当前 Helix pin `98c522a`。

开放：

- generated Strict Matrix/command contract 与当前 `caadafd2...` formal Matrix 仍需统一回读；
- PC 文档同时存在“26 canonical-green”和“历史 digest 待按新 pin 重绑/runtime-pass
  not collected”，必须由一个 current candidate 的 machine reducer 裁决；
- 现有 PC Green 不能覆盖 Web，也不能替代 Mobile 真机 UI。

### Mobile

已具备：

- FFI batch、MobileImSdk、SurfaceRuntime、QuickJS typed view、Venus bridge；
- 27/27 current contract-green；
- V1 hard-cut/fixture parity 和多 Gate production-linked GTest；
- 当前 Helix pin `98c522a`。

开放：

- `contract-green` 只证明 command/event/ABI 路由；
- 2026-07-16 real-chain 报告绑定 `1d80c90 + epoch7`，不是当前 pin；
- Android/iOS package、Flutter 真机 UI 和当前 epoch 的双用户真实链需重新采集。

### Web

已具备：

- final WASM；
- Rust Browser Fetch、page-session store、ExternalWsBridge、timer、bounded event batch；
- 真浏览器到真 Go 的鉴权、hello、local-first/HTTP fallback、send/echo、断连恢复。

开放：

- 产品 renderer registry；
- V2 Frame 原子 apply、ActionBinding invocation、FrameApplied/ack/resync；
- G01-G27 的产品 UI 与真实证据；
- 浏览器 capability 差异的明确 Matrix 合同。

---

## 5. P0：收敛当前 27 Gate

- [ ] **锁定单一 candidate**：Helix、PC、Mobile、Matrix raw hash、fixture manifest、
  generated command API、Cargo/source lock 全部一致。
- [ ] **修正 PC current-epoch 读模型**：消除 generated Matrix hash 与 Cargo pin/正式
  Matrix 的漂移，重新生成而非手工改 Green。
- [ ] **关闭 Spec 020 总控尾项**：复核 G16b/G16c 已合入事实，完成 GS01-GS04、
  program report、legacy candidate、人工删除选择与删除后回归。
- [ ] **当前 pin 的 PC 聚合运行**：同一 Native candidate、双账号、required fail/skip=0，
  输出 authority/local store/Frame/DOM/trace 可关联 evidence。
- [ ] **当前 pin 的 Mobile 聚合运行**：FFI/QuickJS 与产品真机 UI 分列，禁止用
  production-linked GTest 代替真机。
- [ ] **Web renderer 最小纵切 G01**：只消费 typed Frame、opaque action 和 ack/resync，
  不新增业务 reducer。
- [ ] **G01 三端同窗**：在同一 Matrix/candidate 下首次产生一个可进入 TSCR 的
  `ThreePlatformGreen`。

P0 停止条件：

- 任一 pin/hash 不一致；
- 任一 required 面缺失或使用旧 evidence window；
- Web 仍无产品 renderer；
- 出现 V1 Frame、raw business fallback、平台自造业务终态；
- 同一外部阻塞连续出现三次且不能在当前权限内解决。

---

## 6. P1：扩展 active Gate

- [ ] **G03 媒体真实闭环**：Rust 直连 Java prepare/PUT/complete/readback，current pin 的
  PC/Mobile evidence；Web File/Blob 与 CORS 未实现前保持精确 unsupported。
- [ ] **G17-G21**：频道/消息置顶、公告、presence、条件查询、成员和管理员治理。
- [ ] **G25-G27**：书签、待办/通知/全模块读、公司大群、他人消息推送。
- [ ] 每个 Gate 使用同一模板：Spec -> RED contract -> Helix producer -> canonical
  fixture -> 三端 consumer -> current-pinned real evidence -> TSCR。
- [ ] Matrix 的 `contract_state` 只能由机器验证从 declared 晋级 implemented。

P1 不包含：

- G22 搜索、G23 投票、G24 平均分，除非新 ADR/Matrix epoch 将其重新激活；
- 为尚未出现的业务预建通用插件系统；
- 用 Web 不支持作为理由把业务规则复制回 PC/Mobile。

---

## 7. P2：基座证明与工程化

- [ ] 接入第二个形状不同的业务模块，证明 `Module + register()` 且 core 零改。
- [ ] 多模块共享单写者的并发实测与公平性 tripwire。
- [ ] proxy/TLS/cert、旧 callback fallback 的明确产品决策与测试。
- [ ] TSCR machine reducer：读取 Matrix、三端 source lock 和 evidence manifest，输出
  `x/N@epoch+sha` 与逐 Gate 第一缺面。
- [ ] 文档熵 gate：断链、旧 pin、冲突 Green、ADR/HX-C 数量、Roadmap 历史路线扫描。
- [ ] release gate 执行三端 Rust、final WASM、FFI ABI/portability 与当前 scoped real E2E。

---

## 8. 固定验收命令

共享协议、core、port 或 driver 变化的 phase 验收：

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web
cargo test -p helix-core
cargo test -p helix-im
cargo build --target wasm32-unknown-unknown -p helix-driver-web
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

最后一条必须无输出。文档、单 Gate 或局部 adapter 变更按 blast radius 使用聚焦验证，
不得把无关历史失败扩张为本批次工作。

发布裁决必须另外读取三端 current-pinned evidence；Cargo Green 不等于 TSCR。

---

## 9. 完成定义

一个 Roadmap 项只有同时满足以下条件才能打勾：

- 文件归属和依赖方向正确；
- 生产代码、合同和 fixture 同 pin；
- 聚焦验证通过，required skip/fail 如实记录；
- Mock、real、dual-user、persistence、UI 分面明确；
- 没有 V1/raw fallback；
- `git diff --check` 通过；
- 若声称 ThreePlatformGreen，满足 GOAL §5 全部条件。

否则只能标记 `partial`、`blocked`、`unknown` 或 `stale`。
