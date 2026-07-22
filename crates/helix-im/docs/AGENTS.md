# helix-im V2 Interaction Docs

本文件是 `crates/helix-im/docs/` 的局部 Agent 入口。它不覆盖仓库根
`AGENTS.md` 的架构、验证和提交规则；只规定 **InteractionFrame v2** 任务在本目录
如何建立最小、无 V1 污染的文档上下文。

## 目标与边界

- 领域归属：`helix-im` 拥有 IM 领域事实、Surface scope、权限、`ActionBinding`、
  `SurfaceComposer` 和 `InteractionFrame v2`；`helix-core` 不认识 IM/UI 类型。
- 依赖方向：平台只机械解码、校验、原子应用 Frame 并执行本地 UI；业务裁决始终在
  `helix-im`，不得让 PC/Mobile 从 raw emit 或本地 Store 重建未读、排序、权限或
  `temporaryId` 对账。
- 本目录的 V2 合同描述目标实现，不等同于已实现、已验收或 runtime green。Gate 的
  实际状态只能由代码、回放、跨端 Frame apply 和真实链路证据确认。

## V2 上下文白名单与优先级

为 V2 Agent 建立上下文时，**只读取下列文档**，并按此优先级解决冲突：

1. [`interaction-v2/`](./interaction-v2/)：V2 的逐 Gate 实施合同，是首要设计来源。
2. [`UC01-16-界面投影视图模型与交互帧合同.md`](./UC01-16-界面投影视图模型与交互帧合同.md)：
   Surface、Frame、ActionBinding、Projector 和平台消费的上位合同。
3. [`UC01-27双视角功能与交互说明.md`](./UC01-27双视角功能与交互说明.md)：
   发起者/接收者的产品交互和双视角收敛要求。
4. [`前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](./前端UC投影指令与数据清单-最新代码分析-2026-07-09.md)：
   **仅按需的 V1/旧链字段与迁移前事实参考**；不得作为 V2 设计、下行 wire 或端侧
   fallback 的来源。

除了父级仓库指令和当前任务直接涉及的源码/测试证据外，不读取其他旧文档来拼装 V2
运行时上下文。尤其不得因图省事而把旧 `im:*`、`increment_*`、ProjectionStore 或
V1 payload 文档当成新的业务 UI 合同。

## 最小阅读顺序

1. 先读本文件与 [`README.md`](./README.md)。
2. 必读 [`interaction-v2/00-首批纵切总览与硬切准入.md`](./interaction-v2/00-首批纵切总览与硬切准入.md)。
3. 只读当前 Gate 的分册：`01` 对应 GS01-GS04，`02` 对应 G01，`04` 至 `08` 分别覆盖
   G02-G16c；不要为了一个 Gate 预加载全部历史材料。
4. 变更 Frame wire、Frame Cache、ack/resync、directive、renderer 边界时，必读
   [`03-FrameCache原子替换与三端渲染合同.md`](./interaction-v2/03-FrameCache原子替换与三端渲染合同.md)。
5. 变更 Gate 覆盖、状态或 raw emit 下线时，必读
   [`09-全Gate覆盖矩阵与raw-emit下线清单.md`](./interaction-v2/09-全Gate覆盖矩阵与raw-emit下线清单.md)。
6. 需要全局 Surface/Frame 边界时，按需读取 UC01-16 的对应章节；需要产品交互、双用户
   收敛时，只读 UC01-27 中对应 UC。只有 V1 字段兼容、旧链迁移核验或历史事实比对时，
   才读取第 4 项旧链清单的相关 Gate 段落。

`interaction-v2/flows/` 是 GS01-GS04 和 G01 的运行/证据辅助材料。它帮助定位和验收，
但不能替代 Gate 合同、回放或真链路证据。

## V2 不变量

- V2 只有九个 Surface，含频道级 `message.composer`；新增能力先归属既有 Surface 或
  明确提出新的合同，不能塞进无类型属性包。
- `InteractionFrame v2` 是 `runtime + typed mutations + directives` 的版本化批次；同一
  领域提交影响多个已 attach Surface 时必须原子提交。未知 schema/kind/version
  fail-closed。
- 业务 UI 只在权威事实 `PersistOk` 后由 `SurfaceComposer` 生成。HTTP 2xx、raw WS、
  raw emit 或 `ActionReceipt(accepted)` 都不是最终业务 UI。
- 平台维护只读 Frame Cache 和本地 overlay；草稿、选中文件、菜单、路由、焦点、动画和
  像素滚动不写回业务 VM。端侧不得保留第二套 Message/Channel reducer。
- revision/epoch/actionRef 由 Helix 签发。base revision gap、旧 epoch 或未知 type 时，
  整帧拒绝或请求对应 slot snapshot，绝不回退到旧 raw event。
- raw emit 的物理下线是硬切，不是双写迁移。未实现的 Gate 必须明确标 `planned`，不能
  用 V1 fallback 把它伪装为可用。

## 文档维护规则

- 新增或修改某个 V2 Gate，改它的分册；涉及公共 Frame/Cache 语义时同步 `03`；涉及
  覆盖、状态或 raw emit 清单时同步 `09`。不要只在 README 记录行为合同。
- 状态必须区分“合同已定”“实现中”“已验证”。只有相应代码和证据齐全时才能改写 Gate
  状态，文档覆盖不构成完成证明。
- `UC01-16` 是上位语义合同，`UC01-27` 是产品交互合同；两者与 V2 分册冲突时，先停止
  并澄清，不在 V1 路径中自行寻找折中方案。
- V1 旧链清单只可帮助识别迁移输入、字段或兼容风险。任何由它发现的新需求，必须先写入
  对应 V2 Gate 合同后，才可进入实现。
