# Helix 项目北极星

> 本文是 Helix 的 L2 目标常量。当前实现、运行证据和排期分别以代码、Strict Matrix、
> evidence pack 与 [`ROADMAP.md`](./ROADMAP.md) 为准，历史合同不得反向改写本目标。

## 1. 一句话定义

**Helix 是运行时无关的确定性业务内核：同一业务事实与状态机实现一次，通过版本化
MessageV3 Gate Event 交付给 PC、Mobile 与 Web。**

IM 是首个业务模块，不是内核特例。`helix-core` 只提供 sans-IO 执行壳和 ports；
`helix-im` 持有消息领域状态机；driver 负责 I/O、线程、生命周期和批量事件交付；
平台 UI 负责组件、布局、像素、动画与本地交互状态。

当前架构决策以 Constitution 2.0、ADR-017 与
[`specs/047-im-event-gate-inventory`](../specs/047-im-event-gate-inventory/) 为准。
旧呈现套壳已经退役，不再是目标架构或完成依据。

## 2. 设计不变量

1. **运行时无关**：`helix-core` 不依赖 tokio、sqlx、reqwest、web-sys、tauri 或业务
   schema；业务模块只依赖 core ports。
2. **Sans-IO / 六边形**：core 接收 Tick 并产出 Effect；网络、存储、时钟、线程与文件系统
   位于 driver。
3. **确定性**：时间、随机和 ID 由 host 注入；同一输入剧本产生字节等价输出。
4. **并发不进 core**：core 单线程可运行；`Send` 只在跨线程 adapter 边界增加。
5. **依赖单向**：`platform adapter -> application/module -> core ports`，禁止反向 import。
6. **业务不污染内核**：channel、message、temporaryId 与 MessageV3 payload 属于
   `helix-im`；`helix-core` 只搬运不透明业务 bytes。
7. **业务状态只实现一次**：权限、temporaryId 对账、窗口、排序、未读、重试终态与恢复
   由 `helix-im` 决定，shell 不重建 reducer。
8. **持久事实先于可见终态**：durable event 只能在 matching `PersistOk` 后由本地事实读回
   产生；HTTP 2xx、WS 入队、AcceptedCommand 或内存 mutation 都不是完成。
9. **逐跳可观测**：Action/Observation -> Tick -> Effect -> authority -> Persist ->
   Emit -> driver delivery -> application observation 使用同一 correlation。
10. **跨边界批处理**：FFI/Web 按 step 或 batch 交付 event，不逐条跨边界回调。

## 3. MessageV3 Gate Event 合同

- 唯一业务出站信封为 `Effect::Emit {event,data}`。
- `event` 使用 Gate 注册的稳定名称；`data` 只含已提交存储事实、已观测 wire 字段或明确
  声明的 correlation/query/role 派生字段。
- sender 与 receiver 各自从 tenant-local 持久事实生成绝对态；shell 不消费 delta 补算
  未读、权限、排序或频道摘要。
- terminal event 在 PersistOk 前不得出现；重复、陈旧、错误 scope 或 PersistErr 不得释放
  成功终态。
- Native channel、FFI batch 与 Web bridge 只原样交付同一 bytes，不包装第二层业务协议。
- `.v1` 只是 Spec 047 的本地证据/schema 版本；canonical JSON 必须由真实
  `ImModule::handle` 回放测试生成。
- PC WDIO 与 Mobile GTest 只能 byte-exact 镜像 canonical JSON，不能手写或补字段。

## 4. 分层边界

| 归属 | 可以拥有 | 不可以拥有 |
|---|---|---|
| `helix-core` | Tick、Effect、port、Module host、确定性调度 | IM schema、平台 UI、HTTP/DB client |
| `helix-im` | Gate 状态机、持久屏障、事件字段、权限、未读、恢复与双视角 | DOM、Widget、CSS、像素、平台句柄 |
| driver/host | HTTP/WS/SQLite、timer、线程、FFI/WASM/Tauri bridge、批量投递 | 业务 fallback、消息对账、权限裁决 |
| platform UI | 组件、布局、动画、输入焦点、临时展示缓存 | raw event reducer、权威字段补算、跨 event join |

判据：**多个平台应得到相同业务结果的规则归 `helix-im`；平台如何展示和执行 I/O 归边缘。**

## 5. 唯一北极星指标

### 三端同核 Gate 闭环率（TGCR）

```text
TGCR(epoch, matrix_sha) =
  同一候选版本中 Helix canonical + PC + Mobile + Web 全部 Green 的 active Gate 数
  -------------------------------------------------------------------------
  Strict Matrix 在该 epoch 下 lifecycle=active 的 Gate 数
```

一个 Gate 只有同时满足以下条件才进入分子：

1. Matrix、Helix revision、三端 pin 与 canonical hash 一致；
2. Helix 真实 command/inbound 经 authority、PersistOk 后产出唯一 MessageV3 event；
3. 三端原样消费同一 event bytes，未保留旧协议或平台业务 reducer；
4. required fail=0、required skip=0，且证据处于同一有界运行窗口；
5. Mock、fixture、HTTP 200、WS 帧、单库读回或单端 Green 均不能单独晋级。

Spec 047 当前版本只完成 Helix producer、测试、物理下线与 canonical 生成；PC/Mobile 同步
明确为 `NOT_SYNCED_BY_SCOPE`，因此不得提前宣称 TGCR Green。

## 6. 成功标准

- 30 个当前 Gate 都有真实 `ImModule::handle` 回放、双视角 canonical JSON 与稳定 provenance；
- Helix 当前生产、测试、Matrix 和合同对退役 Frame/Surface 业务引用为 0；
- 每个 migrated Gate 的旧 V1/V2 producer、parser、fixture、fallback 与 capability 已物理删除；
- 同一输入剧本跨 native/FFI/Web 产生字节等价 MessageV3 event；
- core 零 runtime/业务/UI 泄漏，热路径和跨边界批处理持续通过 HX-C 护栏；
- 后续三端消费只做 byte-exact mirror 与宿主验证，不新增业务状态机。

## 7. 明确禁止

- 在 core 引入平台 runtime、ambient time/random 或业务 schema。
- 在 PC/Mobile/Web 复制 ProjectionStore、temporaryId 对账、未读或业务 fallback。
- 用旧呈现套壳包装 MessageV3 event，或先转旧合同再转 event。
- 用 V1 normalize、raw compatibility reducer 或双写临时保绿。
- 在 PersistOk 前发布 durable terminal event。
- 用 Mock、fixture、accepted、HTTP 200 或单端 runtime-pass 写成三端 Green。
- 手写 canonical payload，或让证据脚本构造业务字段。

## 8. 防漂移机制

- ADR 位于 [`harness/decisions.md`](./harness/decisions.md)，当前实施活账位于
  [`ROADMAP.md`](./ROADMAP.md)。
- Strict Matrix、Spec、代码和 evidence pack 的状态不得手工复制成另一套权威表。
- 新 Gate 先更新 manifest/Matrix；状态变化只由测试、删除清单和机器证据派生。
- 任何与本文不变量冲突的改动必须先停止并请求决策。
