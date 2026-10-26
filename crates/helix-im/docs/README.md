# helix-im 当前文档入口

> 当前生产协议已经硬切为 MessageV3 typed event：UI 下行是
> `Effect::Emit -> EventSink`，不再存在 `InteractionFrame`、`Surface` 或 V1 compatibility
> 套壳。`interaction-v2/` 与旧 Frame 合同仅保留为历史设计材料，不得当作当前运行时真源。

## 当前运行时流程

| 文档 | 用途 |
|---|---|
| [`频道同步分页协议.md`](./频道同步分页协议.md) | MessageV3 hello increment ready、account-local channel 分页与 complete 协议。 |
| [`消息发送全流程/在线与离线发送消息全流程.md`](./消息发送全流程/在线与离线发送消息全流程.md) | 结合 Helix 与现役 Go server 源码，说明 `im_send_message`、Persist/HTTP/timer/PortReply、WS authority、sender/receiver 未读、离线补偿与手动 retry。 |
| [`消息发送全流程/在线与离线发送消息全流程.mmd`](./消息发送全流程/在线与离线发送消息全流程.mmd) | 同一链路的 Mermaid 图源。 |

## 同步与成员绝对态边界

- 公司身份不限制已加入群的展示/读取。持久频道按 `auth_user_id` 隔离，读回仍校验有效成员；未知频道及其他账号资料继续拒绝。
- 已提交频道请求的 `companyId` HTTP header 取该频道真实租户，账号与凭据不变；全局请求与 Java/OSS 请求保留原身份。群租户不能用来判断人员资料所属公司。
- 跨公司水合不得构造归档事实；真正 `close/deleteAt` 与成员退出终态仍保留。

- `im_query_dialog_list` 在频道分页后按频道集合读取当前认证用户的成员状态，
  两阶段共用 `DialogListQuery` 身份与 typed Result；成员回读前不发布旧未读数。
  未读 count/anchor/read-seq/revision 复用 `channel_sync` 的绝对态校验，
  成员主键和公司字段不得覆盖频道归属，成员置顶合并后按原三键稳定排序。
- gate 补拉、主动同步和满批续拉共用有界调度器；对应原子提交回执到达前，
  同一频道的后续意图留队，发出请求时读取已提交游标。失败不得推进游标。
- `too_long` 的原子批次只包含写入，读回在 `PersistOk` 后单独派发。
  RuntimeAuth 重置清除在途 owner，迟到回执不能重新发起旧同步或结算旧查询。

源码入口：`src/query/dispatch.rs`、`src/event/read.rs`、`src/sync/scheduler.rs`、
`src/sync/flow.rs`、`src/port_reply/dispatch.rs`；回归入口为 `p6_query_test`、
`p4_sync_scheduler_test`、`gate_test` 和 Host 的 `too_long_reload_atomic`。
同步护栏见 [HX-C008](../../../docs/harness/HX-C008-sync-pull-wire-contract.md)。

## 合并转发媒体投影

当前合并消息只交付有界 `props.forward` 卡片，不再保存或递归投影来源全文。
详情由 `im_forward_detail` 按 root/path/offset 读取单页；媒体条目从该页的 props 投影，
fileName 由 Helix 归一为 name，嵌套转发仍只输出子卡片。
命令参数、取消边界与测试入口统一见 [`crate README`](../README.md#合并转发投影)，
源码真源为 [`query/render_ready/forward.rs`](../src/query/render_ready/forward.rs) 和
[`query/forward_detail.rs`](../src/query/forward_detail.rs)。

这些入口只说明当前实现，不代表本轮已运行回放、真实宿主或 UI 验收。

下面的 V2 内容是硬切前历史索引，只用于追溯设计决策，不参与当前 MessageV3 实施或 Green 判定。

## 历史 InteractionFrame v2 入口

> `RETIRED_HISTORICAL`：这是 V2 Agent 的历史上下文入口。先读 [`AGENTS.md`](./AGENTS.md)，
> 再按 Gate 追溯合同；不要以旧 V1 文档、raw emit 或端侧 ProjectionStore 建立当前运行时上下文。
> 当前实施合同与 Gate 分母请改读 Spec 047。

## 历史索引中的四类来源

| 优先级 | 文档 | 用途 |
|---|---|---|
| 1 | [`interaction-v2/`](./interaction-v2/) | V2 的逐 Gate 实施合同、Frame 语义、覆盖台账和运行辅助材料。 |
| 2 | [`UC01-16-界面投影视图模型与交互帧合同.md`](./UC01-16-界面投影视图模型与交互帧合同.md) | 上位的 Surface、ViewModel、ActionBinding、Projector、InteractionFrame 与 SurfaceRuntime 边界。 |
| 3 | [`UC01-27双视角功能与交互说明.md`](./UC01-27双视角功能与交互说明.md) | 发起者/接收者的产品交互、权限差异和双视角收敛目标。 |
| 4 | [`前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](./前端UC投影指令与数据清单-最新代码分析-2026-07-09.md) | **仅按需的 V1/旧链前置参考**：字段、旧输入和迁移风险核验；不参与 V2 默认上下文，也不定义 V2 wire/fallback。 |

当前代码、回放、测试和真实链路仍负责验证实现事实；它们不能被文档的“已覆盖”措辞替代。

## 历史 Gate 路由

| Gate | V2 分册 | 先解决的问题 |
|---|---|---|
| GS01-GS04 | [`01-GS01-GS04-同步到Frame完整时序与数据.md`](./interaction-v2/01-GS01-GS04-同步到Frame完整时序与数据.md) | hello、离线补偿、gap、陌生频道 hydration 如何在 `PersistOk` 后形成 Frame。 |
| G01 | [`02-G01-文本发送到Frame完整时序与数据.md`](./interaction-v2/02-G01-文本发送到Frame完整时序与数据.md) | `message.composer` capability、temporary identity、Sending/Sent/Failed 与双用户收敛。 |
| Frame/端侧消费 | [`03-FrameCache原子替换与三端渲染合同.md`](./interaction-v2/03-FrameCache原子替换与三端渲染合同.md) | typed decode、epoch/revision、原子 apply、ack/resync、directive 与 overlay 边界。 |
| G02-G03a | [`04-G02-G03a-文档媒体上传与重试.md`](./interaction-v2/04-G02-G03a-文档媒体上传与重试.md) | 文档、媒体过程态与同一 temporary row 的重试。 |
| G04-G07 | [`05-G04-G07-消息变更转发回应与加急.md`](./interaction-v2/05-G04-G07-消息变更转发回应与加急.md) | 撤回、转发、reaction、加急及确认。 |
| G08-G12 | [`06-G08-G12-定时窗口定位与回复.md`](./interaction-v2/06-G08-G12-定时窗口定位与回复.md) | 定时、窗口、定位和 reply 的有界读取与投影。 |
| G13-G14a | [`07-G13-G14a-未读回执与模板确认.md`](./interaction-v2/07-G13-G14a-未读回执与模板确认.md) | Viewer read、逐消息 receipt、模板确认的独立事实和 Surface。 |
| G15-G16c | [`08-G15-G16c-建群话题与频道治理.md`](./interaction-v2/08-G15-G16c-建群话题与频道治理.md) | 建群、话题、关闭、退出和成员/频道治理。 |
| 全 Gate/硬切 | [`09-全Gate覆盖矩阵与raw-emit下线清单.md`](./interaction-v2/09-全Gate覆盖矩阵与raw-emit下线清单.md) | Gate 状态、九项迁移闭环与 legacy raw emit 的物理下线范围。 |

G04-G16c 的 Go HTTP/WS、Persist 屏障与完整 Frame 数据展开见
[`interaction-v2/完整时序和数据/README.md`](./interaction-v2/完整时序和数据/README.md)。该目录是源码对齐说明，
不替代本表的目标合同或 `09` 的验证状态。

每次 V2 任务先读 [`00-首批纵切总览与硬切准入.md`](./interaction-v2/00-首批纵切总览与硬切准入.md)，
再按表选择一个 Gate 分册。改 Frame wire/Cache/ack/resync 时加读 `03`；改 Gate 状态或
raw emit 时加读 `09`。`flows/` 只服务 GS01-GS04 和 G01 的运行、证据与人工介入，不替代
其实现合同。

## 历史 V2 共同模型

V2 的唯一业务 UI 下行是版本化 `InteractionFrame v2`：`runtime`、typed surface
mutations 与一次性 `UiDirective` 在同一批次中交付。业务事实先经 Helix 持久化；只有
`PersistOk -> SurfaceComposer -> InteractionFrame` 才能改变可见业务 UI。HTTP 2xx、raw
WS、legacy emit 和 accepted receipt 都不能代替该链路。

九个 Surface 为：

`message.left-dialog-list`、`message.unread-summary`、`message.timeline`、
`message.thread-detail`、`message.channel-summary`、`message.member-roster`、
`message.topic-list`、`message.participant-receipts`、`message.composer`。

`helix-im` 负责领域事实、影响面、排序、未读、权限、epoch/revision 和 action grant；PC/Mobile
只做强类型 decode、Frame Cache 原子替换、renderer 映射与本地 overlay。未知类型、旧 epoch
或 base revision gap 必须 fail-closed/resync，不能回退为 raw event reducer。

## 何时才读 V1

只有以下情形可按需打开“前端 UC 投影指令与数据清单”中的对应段落：核对旧字段、识别
raw WS/HTTP 输入、评估迁移兼容风险，或对照旧链的已知事实。它的内容只能解释迁移起点，
不能改变 V2 的 Surface owner、typed union、Persist barrier、Frame 原子性或 raw emit 硬切
策略。发现 V1 未覆盖的新需求时，先补 V2 Gate 合同，再讨论实现。
