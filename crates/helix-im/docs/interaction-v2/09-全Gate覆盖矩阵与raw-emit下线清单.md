# InteractionFrame v2 - 全 Gate 覆盖矩阵与 raw emit 下线清单

> 状态：Review Draft。本文是实现和删除前的账本，不是 runtime green 证明。当前仅完成 V2 合同；任何 Gate 必须在它自己的代码、回放和三端消费证据通过后才可改为已实现。

## 1. 27 Gate 覆盖矩阵

| Gate | 入口 / 关键事实 | V2 Surface / Frame | 分册 | 当前状态 |
|---|---|---|---|---|
| GS01 | SessionStart、初始 sync facts | `runtime` + 已 attach user/channel surface | `01` | 实现优先级，合同已定。 |
| GS02 | 增量 WS/sync facts | impact policy 对应的一组 Surface | `01` | 实现优先级，合同已定。 |
| GS03 | gap / cursor recovery | `runtime.syncing` + 补偿后重组的 surface | `01` | 实现优先级，合同已定。 |
| GS04 | EnterChannel、陌生频道 facts | `channel-summary + composer` bundle，timeline 按需 | `01` | 实现优先级，合同已定。 |
| G01 | `message.create`、Message/temporary mapping | composer + timeline + dialog + unread/summary | `02` | 实现优先级，合同已定。 |
| G02 | 文档消息 create | 同 G01 的 typed content row | `04` | planned。 |
| G03 | 文件选择/上传 | upload lifecycle + typed row | `04` | planned；真实文件输入/upload 本轮 deferred。 |
| G03a | 媒体消息、上传结果 | timeline/thread/dialog preview | `04` | planned。 |
| G04 | 撤回 | timeline/thread/dialog + receipts when attached | `05` | planned。 |
| G04a | 撤回后的状态/同步 | 同 G04 的 authoritative facts | `05` | planned。 |
| G05 | 转发 | source/target timeline、dialog、composer policy | `05` | planned。 |
| G06 | reaction | message row/thread row/可能的 dialog preview | `05` | planned。 |
| G07 | 加急 | message row、dialog、unread summary | `05` | planned。 |
| G08 | 定时发送 | dialog、summary、必要时 timeline | `06` | planned。 |
| G09 | 取消定时 | dialog、summary、scheduled row | `06` | planned。 |
| G10 | 首屏消息窗口 | timeline snapshot | `06` | planned。 |
| G10a | 向上补历史 | timeline bounded replacement | `06` | planned。 |
| G11 | 定位原消息 | timeline + `RevealMessage` directive | `06` | planned。 |
| G12 | 打开/更新回复 | thread detail + root timeline row | `06` | planned。 |
| G13 | 频道 read boundary | dialog + unread summary + timeline | `07` | planned。 |
| G14 | 消息 read receipt | row summary + attached receipts | `07` | planned。 |
| G14a | 模板确认 | row summary + attached receipts | `07` | planned。 |
| G15a | 建群 | creator/member dialog、unread；再走 GS04 | `08` | planned。 |
| G15 | 建话题 | topic list、summary、root timeline/thread | `08` | planned。 |
| G16 | 关闭频道 | summary、dialog、composer、attached channel surfaces | `08` | planned。 |
| G16b | 退出频道 | user dialog/unread + detach channel surfaces | `08` | planned。 |
| G16c | 频道/成员/topic 更新 | 由 impact policy 精确选择 Surface | `08` | planned。 |

`GS01-GS04 + G01` 是本轮实现的唯一代码优先级；其余 Gate 的目标形状已经定义，但不因为有文档而具有下行呈现。G03 的真实文件选择、opaque host file reference 和上传 transport 更明确延后。

## 2. 每个 Gate 都必须交付的 V2 闭环

以下九项缺一项，就不算该 Gate 已迁移：

1. `ActionBinding` 或明确的 `HostObservation` 输入，带 scope、epoch/revision、actor 与 input contract；
2. `Tick::Command` 或 inbound / port reply 的确定性归一；
3. 所需 `Effect`（HTTP、WS outbound、Storage、timer、directive），且 port hop 可观测；
4. authoritative 远端或本地事实的解析和领域裁决；
5. 一笔明确的 `PersistAtomic`，其 `PersistOk` 是业务 Frame 的唯一前置；
6. 所有受影响 Surface 的完整 typed VM/Row projector；
7. `SurfaceComposer` 的 attach-aware impact policy、稳定排序、revision/action grant 刷新；
8. 语义完整的 `InteractionFrame v2`，包含 runtime/mutations/directives 和正确 transactionId；
9. PC/Mobile 的同 wire Frame Cache 原子 apply、ack/resync、renderer，以及该 Gate 的回放/端到端证据。

第 9 项不是把业务逻辑复制进两个客户端：端侧只解码、验证、原子缓存替换、渲染和回报；所有“哪些 Surface 受影响”的判断仍在 `helix-im`。

## 3. raw emit 物理下线范围

已接受的策略是：**10 份合同经人工 review 后，直接物理删除全 IM legacy business raw emit producer；不为未迁移 Gate 保留 consumer fallback。** 这会让 G02-G16c 在各自 V2 producer 未实现前没有业务下行呈现，属于已知且接受的硬切风险。

下表是按旧语义归类的删除范围。执行删除时必须以当前源码 `rg` 盘点为准，逐个记录 producer、consumer、替代 Frame Gate 和测试变化；不能凭本文名称做盲删。

| legacy raw 语义族 | 典型旧事件 / producer 形态 | V2 替代 |
|---|---|---|
| 消息局部状态 | `im:post:sending`、`im:post:received`、`im:post:updated`、`im:post:deleted`、`im:post:read`、`im:post:readers`、`im:post:located` | G01-G14a 对应的 timeline/thread/dialog/receipt mutation。 |
| 频道和会话列表 | `im:channel:increment`、`im:channel:update`、`im:channel:update-by-post`、`im:channel:created`、`im:channel:closed`、`im:channel:left`、`im:channels:loaded` | GS01-GS04、G15-G16c 的 dialog/summary/unread/composer mutation。 |
| 成员、话题、回复 | `im:channel:members`、`im:channel:replies` 及同类 projection event | roster/topic/thread/timeline 的 typed mutation。 |
| 运行态和补偿信号 | `im:ready`、`im:health`、`im:sync:gap` 及同类控制 emit | `InteractionFrameV2.runtime`；不再独立驱动业务 store。 |
| 通用增量 / 透传投影 | `increment_*`、raw WS/HTTP JSON 再 emit、以 payload tag 分流的总线事件 | inbound facts -> PersistOk -> `SurfaceComposer` -> v2 Frame。 |

保留的不是 legacy business emit，而是 V2 传输合同本身：`im:interaction:frame`（替换为 v2 schema）、`im:interaction:action-receipt`（替换为 v2 ack 语义）以及 non-business platform lifecycle 通道。它们也必须有明确 schema version，不能借旧 event 名承载未版本化 JSON。

## 4. 删除顺序与不可回退原则

```text
人工 review 10 份合同
  -> 建立当前 producer/consumer 清单
  -> 删除所有 legacy IM business raw producer
  -> 删除 raw consumer、ProjectionStore 业务写入和兼容分支
  -> 只保留 v2 Frame Cache 入口
  -> GS01-GS04 + G01 先实现、回放、三端验证
  -> 逐 Gate 实现 G02-G16c
```

这不是“双写过渡”。一旦删除动作开始，不允许再增加 raw emit 作为临时回退。发现 V2 缺口时，修复该 Gate 的 Core/Effect/Persist/Composer/Frame 合同，而不是把旧总线接回。

本次仅产出文档，**未执行 producer/consumer 删除，也没有把任一 Gate 标记为 runtime green**。物理删除须等用户 review 明确通过后单独实施和验证。

## 3.1 SPEC-001 V2 consumer gate and current raw-owner inventory (T048)

The GS01-GS04 candidate has a fail-closed consumer rule: `InteractionFrameV2::decode` is the
only accepted V2 recovery input. A V2 envelope cannot be decoded as `InteractionFrame`, and an
invalid V2 envelope does not become a legacy `im:*` projection. The Rust contract test
`v2_consumer_gate_has_no_v1_or_raw_projection_fallback` keeps this boundary explicit; PC and
mobile retain their independently strict schema decoders.

This is **deletion readiness inventory only**, captured against the current source tree. These
legacy producers remain live and are not a fallback for GS01-GS04:

| Owner | Current raw producer family | Required V2 replacement before separately authorised deletion |
|---|---|---|
| `crates/helix-im/src/channel/gate/projection.rs` | `im:post:*` from contiguous WS gate application | G01-G14a typed timeline/thread/dialog/receipt Frames |
| `crates/helix-im/src/ws/handlers/post/parse.rs` | `im:post:received` unsequenced ingress | G01 typed post/frame path |
| `crates/helix-im/src/ws/handlers/posts_update.rs` | `im:post:batch-updated` and delete projection | G04-G07 typed affected-surface Frames |
| `crates/helix-im/src/increment_hydration.rs` | `im:channel:increment` | GS01-GS04 dialog/summary/unread/composer Frames |
| `crates/helix-im/src/port_reply/dispatch*.rs` | channel/member/reply/read raw projections | G13-G16c typed surface Frames |
| `crates/helix-im/src/client_api/capabilities.rs` | public legacy event capability list | Cutover must remove the matching producer and consumer entries together |

No row above is approval to remove code. The required four-plane source pin, readonly database
evidence and real-chain verification are still recorded by SPEC-001 T049/T050 before a human may
authorise the independent physical deletion change.

## 5. 审查和实施准入

### 文档自审

- 顶层 Gate 合同必须正好十份：`00` 至 `09`；`flows/` 仅容纳对应的运行流程图与证据手册；
- 上表 27 个 Gate 各出现一次，且均有分册、输入/事实和 Surface 目标；
- 九 Surface（尤其 `message.composer@Channel`）必须有唯一 owner/scope；
- `InteractionFrame v2` 必须有 `runtime + mutations + directives`，且 Cache 原子 apply；
- 所有 `planned` Gate 不得写成已实现、已发送 Frame 或三端绿色。

### 代码迁移后才可做的验证

1. 单 Gate 回放：断言 `Tick -> Effect -> PersistOk -> Frame` 的顺序和 actionRef stale 行为；
2. persist barrier 破坏测试：让 Persist 失败，断言没有业务 Frame；
3. Frame Cache 原子性：拒绝一个 mutation 时整帧不提交、无 directive 执行；
4. PC/Mobile 同一 golden Frame：断言 Cache snapshot、受影响 Surface 和 action binding 语义一致；
5. 真 Go 链路：GS01-GS04/G01 后再分别验 HTTP、WS、storage 和实际渲染，不能用静态文档代替。

这些准入把“文档覆盖完整”与“运行时交付完成”明确分开，避免一个漂亮矩阵掩盖尚未迁移的下行空洞。
