# Helix 项目北极星

> 本文是 Helix 的 L2 目标常量，约束产品方向、架构边界和完成判据。
> 每次会话按 `AGENTS.md -> GOAL.md -> SESSION.md -> harness/README.md` 加载。
> 当前实现、运行证据和排期分别以代码、Strict Matrix/evidence pack 与
> [`ROADMAP.md`](./ROADMAP.md) 为准；它们不得反向改写本文的不变量。

---

## 1. 一句话定义

**Helix 是运行时无关的确定性业务与交互内核：业务状态、业务动作和跨端 UI
语义只实现一次，由 PC Tauri、Mobile FFI 和 Web 三个平台机械执行。**

IM 是首个业务模块，不是内核特例。`helix-core` 只提供 sans-IO 执行壳和 ports；
`helix-im` 持有消息领域状态机与 InteractionProtocol；平台 driver 负责 I/O、线程、
生命周期和协议转换；平台 UI 负责组件、布局、像素与动画。

截至 2026-07-27：

- native/FFI 已产品化并共用 host SQLite/HTTP/WS 与 engine loop；
- `helix-driver-web`、Browser Fetch、page-session store、浏览器 WS bridge、final WASM
  gate 和真浏览器到真 Go 的鉴权、send/echo、断连恢复已经落地；
- InteractionFrame V2、typed Surface、opaque ActionBinding、revision/epoch、ack/resync、
  Persist barrier 与 G01-G16c 的生产合同已经进入代码和 canonical fixture；
- PC 与 Mobile 当前都 pin Helix `98c522a3621cc599c134cb78b9e4656668eb67bb`；
- Web 仍只有平台 driver 与最小 browser fixture，没有产品级三端统一 renderer；
- 当前证据分散在不同 revision、contract epoch 和运行窗口，不能据此宣称全部 Gate
  已完成三端同核闭环。

---

## 2. 设计不变量

1. **运行时无关**：`helix-core` 不依赖 tokio、sqlx、reqwest、web-sys、tauri 或具体
   UI framework。业务模块只依赖 core ports。
2. **Sans-IO / 六边形**：core 只接收 Tick 并产出 Effect；网络、存储、时钟、线程和
   文件系统全部位于 driver。
3. **确定性**：时间、随机和 ID 由 host 注入；同一输入剧本必须产生字节等价输出。
4. **并发不进 core**：core 单线程可运行；`Send` 只在需要跨线程的 adapter 边界增加。
5. **依赖单向**：`platform adapter -> application/module -> core ports`，禁止反向 import。
6. **业务不污染内核**：channel、message、temporaryId、ViewNode 和 UiDirective 属于
   `helix-im`；`helix-core` 只搬运不透明业务载荷。
7. **能力可替换**：Storage、HTTP、WS、Timer、EventSink 独立实现；feature 只能 additive。
8. **交互语义一次实现**：权限、temporaryId 对账、窗口、排序、未读、fallback 和动作
   完成条件由 `helix-im` 决定，各端不得重建业务 reducer。
9. **持久事实先于可见终态**：声称 durable 的 UI 结果必须经过 matching PersistOk；
   HTTP 2xx、WS 入队、AcceptedCommand 或内存 mutation 都不是完成。
10. **逐跳可观测**：Action/Observation -> Tick -> Effect -> authority -> Persist ->
    InteractionFrame -> apply/ack -> UI 必须由同一 correlation 串联。

---

## 3. Headless 边界

Headless 不等于“只能吐扁平 ViewModel”。它表示业务内核不依赖 renderer、窗口系统、
组件框架和 I/O runtime，但可以拥有完整的平台无关呈现状态机与交互图。

| 归属 | 可以拥有 | 不可以拥有 |
|---|---|---|
| `helix-core` | Tick、Effect、port、Module host、确定性调度 | IM schema、ViewNode、DOM、Widget、HTTP client |
| `helix-im` | SurfaceModel、typed VM、ActionBinding、UiDirective、业务动作与完成条件 | Angular Component、Flutter Widget、DOM tag、CSS、像素、平台句柄 |
| driver/host | HTTP/WS/SQLite、timer、线程、FFI/WASM/Tauri bridge、批量投递 | 业务 fallback、消息对账、权限裁决、客户端自造业务终态 |
| platform UI | renderer registry、组件、布局、动画、原生句柄、用户输入采集 | raw event 业务 reducer、命令选择、权威字段补算 |

判据：**多个平台应得到同一业务呈现与动作结果的规则归 `helix-im`；平台如何画、
如何动、如何执行 I/O 的实现归平台。**

---

## 4. InteractionProtocol 不变量

- 下行 `InteractionFrameV2` 与上行 `ActionInvocationEnvelope` 是独立版本合同；前者
  `schemaVersion=2`，后者当前 `schemaVersion=1`，不得按名称批量升级或删除。
- 一个 Frame 是原子 UI 事务；任一 mutation 非法时整帧拒绝，directive 只在提交后执行。
- `surfaceEpoch` 隔离新旧生命周期，`revision/baseRevision` 检测重复、乱序和缺口。
- `ActionBinding.actionRef` 是 actor/scope/epoch/revision 绑定的 opaque capability；
  shell 只提交 actionRef 与允许的 input。
- 无用户动作的 WS、sync、timer 和 port reply 使用明确 HostObservation/Inbound，不伪造
  ActionBinding。
- 持久业务结果同时进入可重放 Surface/VM；一次性 directive 不能成为唯一权威。
- FFI/Web 按 Frame 或 step 批量过边界，不逐 mutation 回调。
- V1 Frame、raw business event 和平台 fallback 必须 fail-closed；合同 fixture、Mock、
  accepted、HTTP、WS、存储、Frame、UI 和双用户证据互不替代。

---

## 5. 唯一北极星指标

### 三端同核闭环率（TSCR）

`TSCR`（Tri-platform Same-core Closure Rate）衡量“一个业务 Gate 是否真正只在 Helix
实现一次，并在 PC、Mobile、Web 三端完成同一权威闭环”。

```text
TSCR(epoch, matrix_sha) =
  当前证据窗口内 ThreePlatformGreen 的 active Gate 数
  ----------------------------------------------------
  Strict Matrix 在该 epoch 下 lifecycle=active 的 Gate 数
```

当前基线：

- Strict Matrix：epoch `11`
- raw SHA-256：`caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`
- 分母：`43` 个 active Gate；G22/G23/G24 为 excluded，不进入分母
- 当前可发布值：`0/43 = 0%`

当前值为 0 不是说 Helix 没有实现，而是尚无任一 active Gate 同时具备**同一 epoch、同一
Helix revision、同一有界运行窗口**下的 PC、Mobile、Web 三端真实证据。PC 的历史
canonical digest、Mobile contract-green、旧 real-chain 报告、Helix fixture 和 Web
平台 smoke 都是有效领先证据，但不能拼接成 TSCR 分子。

一个 Gate 只有同时满足以下条件才是 `ThreePlatformGreen`：

1. Matrix lifecycle 为 active，contract state 为 implemented，Matrix hash 与三端 pin 一致；
2. Helix 直接 command/observation 经过 Effect、authority 和 matching PersistOk，产出
   canonical InteractionFrame V2；
3. PC Native/Tauri、Mobile FFI/QuickJS/产品 UI、Web final WASM/产品 renderer 都机械
   apply 同一语义 Frame，并通过 opaque action/ack/resync 完成交互；
4. command、effect、transport、local store、remote store、projection、UI、trace 八面
   durable evidence 在同一 correlation 窗口内齐全；
5. required fail=0、required skip=0，`v1FrameProducer=0`、
   `v1FrameConsumer=0`、`rawBusinessRenderFallback=0`；
6. Mock、fixture、HTTP 200、WS 帧、单库读回、DOM 截图或单端 Green 均不能单独晋级。

算法必须 fail-closed：缺失、过期、hash/pin 不一致、窗口不可关联均按 0 计。分母随
contract epoch 版本化，指标必须写成 `x/N@epoch+matrix_sha`；epoch 变化时不得把前后
百分比直接比较而不解释新增、排除或拆分的 Gate。

机器真源：

- 分母与合同面：`crates/helix-im/contracts/strict-matrix.v1.json`
- 证据规则：`crates/helix-im/docs/Strict Matrix/README.md`
- 三端总控：`specs/020-ifv2-three-repo-uc-control/`
- V2 硬切：`specs/018-interaction-frame-v2-cutover/`

---

## 6. 成功标准

最终产品成功不是“所有代码已写”或“某端 100%”，而是：

- `TSCR = 43/43@epoch11`，或在新 epoch 下为全部 active Gate；
- 三端业务差异只来自明确的平台 capability，不来自复制的业务逻辑；
- 加新业务模块只实现 `Module + register()`，core 不改；
- 加新平台只实现 ports、bridge 和 renderer registry，core/业务模块不改；
- 同一输入剧本跨 native/FFI/Web 产生字节等价 Frame；
- 所有 durable UI 终态都能追溯到 authority、PersistOk、Frame 与 apply/ack；
- core 零 runtime/业务/UI 泄漏，热路径与跨边界批处理持续通过 HX-C 护栏。

---

## 7. 明确禁止

- 在 core 引入平台 runtime、并发假设、ambient time/random 或业务 schema。
- Web 只持久化 cursor 而不持久化其声明已覆盖的数据。
- PC/Mobile/Web 各写一套 ProjectionStore、temporaryId 对账或业务 fallback。
- 把 semantic renderer key 绑定具体组件、Widget、DOM 或样式名。
- 用 V1 normalize、raw event reducer 或双写“临时保绿”。
- 把 Mock、fixture、contract-green、accepted、HTTP 200、单端 runtime-pass 写成 TSCR Green。
- 为“通用性”预建没有第二个真实模块使用的插件框架。

---

## 8. 防漂移机制

- 本文只记录目标常量、边界和 TSCR 定义，不维护任务流水。
- ADR 在 [`harness/decisions.md`](./harness/decisions.md)，当前实施活账在
  [`ROADMAP.md`](./ROADMAP.md)，战略全景在 [`ROADMAP-full.md`](./ROADMAP-full.md)。
- Strict Matrix、Spec、代码和 evidence pack 的状态不得手工复制成另一套权威表。
- 新决策先写 ADR；新 Gate 先更新 Strict Matrix；状态变化只由机器证据派生。
- 任何与本文件不变量冲突的改动必须先停止并请求决策。
