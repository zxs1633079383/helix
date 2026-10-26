# MessageV3 Phase 1-5 客户端与 Helix Gate 时序图

阅读或修改本目录前先看 [`AGENTS.md`](./AGENTS.md)；本 README 承载 92 Gate 的分母、
Phase/Domain 索引和文件链接，具体 Gate 规则按局部 AGENTS 的渐进式披露顺序下钻。

2026-10-09 源码对齐记录见 [本轮交接](../交接/三仓流程图对齐/handoff.md)。
图首 contract 与覆盖字段是合同登记快照；实际命令、路径与终态以当前源码注释为准。
`im:*` 是 Helix 原始事件；`postReceived` 等是客户端 adapter 的 kind，
`POST` 等是客户端 action，不得混作 EventSink 原始出口。
读结果同样可经 `Effect::Emit` 异步回传；无业务广播不代表无结果事件。

本目录以客户端仓根 `/System/Volumes/Data/workspace/angular/visual/cses-client`
的 Phase1～5 manifest 为机器真源：Phase1～4 为 80 Gate，Phase5 为 12 Gate，总计 **92 Gate**。
按 `Phase → Domain → Gate` 组织 Mermaid 时序图。每张图都沿用
[发送消息参考图](./发送消息.mmd)的逐跳粒度，描述 Angular/Port、Tauri/FFI/Web、Helix
Tick/Effect/PortReply、SQLite/Go HTTP/WS 和最终 raw event 或 Command Result。

说明：客户端合同仍保留完整 92 Gate；Helix Native real-chain runner 为独立的可执行子集，既有选择表
执行 83 个（32/16/15/12/8），历史选择表中 5 个 `NOT_APPLICABLE` 与 4 个 `NOT_IMPLEMENTED` 不进入
real-chain/CI 选择集。两套分母不可互相替代。Category 当前已存在源码实现，不能从 runner 排除状态推导未实现；
该选择表不构成本轮执行证据。

## 分母与领域

| Phase | 客户端阶段 | Domain | Gate 数 |
|---|---|---|---:|
| Phase1 | 发送、上传与真实建群兼容基线 | channel、post、upload、draft、schedule、announcement | 33 |
| Phase2 | 读取、时间线与同步 | read、timeline、sync | 20 |
| Phase3 | 频道与成员 | channel、channel_member | 15 |
| Phase4 | 扩展消息操作与检索 | pin、bookmark、search、announcement | 12 |
| Phase5 | 接龙 | chain、text_chain、category_chain、acceptance | 12 |
| **合计** | 5 个 Phase | 17 个 Phase 内领域目录 | **92** |

> Phase1～4 的 80 Gate 与 Phase5 的 12 Gate 都来自客户端 manifest；Helix 的其他实现清单
> 不能替代该分母。P1 的公告保存（G07b）属于 Phase1 复用能力，
> P4 的 G07a/G07c/G07d 是公告查询、删除和确认收到。

## 目录约定

```text
docs/mmd/
├── Phase1/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
├── Phase2/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
├── Phase3/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
├── Phase4/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
└── Phase5/<domain>/Gate-<gate-suffix>-<中文名称>.mmd
```

领域目录严格按客户端 manifest：
- Phase1：channel、post、upload、draft、schedule、announcement
- Phase2：read、timeline、sync
- Phase3：channel、channel_member
- Phase4：pin、bookmark、search、announcement
- Phase5：chain、text_chain、category_chain、acceptance

## Gate 索引

### Phase1：发送、上传与真实建群兼容基线

#### channel（1 Gate）

- [MV3-G08a 创建群聊](<./Phase1/channel/Gate-08a-创建群聊.mmd>)

#### post（19 Gate）

- [MV3-G01a 发送文本](<./Phase1/post/Gate-01a-发送文本.mmd>)
- [MV3-G01b @所有人（TEMPLATE 壳 + 收到确认）](<./Phase1/post/Gate-01b-@所有人（TEMPLATE-壳-+-收到确认）.mmd>)
- [MV3-G01f 发送文档](<./Phase1/post/Gate-01f-发送文档.mmd>)
- [MV3-G01h 发送任务卡](<./Phase1/post/Gate-01h-发送任务卡.mmd>)
- [MV3-G01i 创建投票并落消息](<./Phase1/post/Gate-01i-创建投票并落消息.mmd>)
- [MV3-G01j 创建评分活动并发送评分卡](<./Phase1/post/Gate-01j-创建评分活动并发送评分卡.mmd>)
- [MV3-G01k 带回复发送](<./Phase1/post/Gate-01k-带回复发送.mmd>)
- [MV3-G01l 定向可见发送（密聊 / 指定接收人）](<./Phase1/post/Gate-01l-定向可见发送（密聊-／-指定接收人）.mmd>)
- [MV3-G02a 乐观三态回显](<./Phase1/post/Gate-02a-乐观三态回显.mmd>)
- [MV3-G02b 发送超时与失败态](<./Phase1/post/Gate-02b-发送超时与失败态.mmd>)
- [MV3-G02c 重发失败消息](<./Phase1/post/Gate-02c-重发失败消息.mmd>)
- [MV3-G03a 逐条转发](<./Phase1/post/Gate-03a-逐条转发.mmd>)
- [MV3-G03b 合并转发（聊天记录卡）](<./Phase1/post/Gate-03b-合并转发（聊天记录卡）.mmd>)
- [MV3-G05a 撤回消息](<./Phase1/post/Gate-05a-撤回消息.mmd>)
- [MV3-G05b 快捷回复 emoji](<./Phase1/post/Gate-05b-快捷回复-emoji.mmd>)
- [MV3-G05c 加急消息](<./Phase1/post/Gate-05c-加急消息.mmd>)
- [MV3-G05d 加急确认](<./Phase1/post/Gate-05d-加急确认.mmd>)
- [MV3-G05e 模板已收到](<./Phase1/post/Gate-05e-模板已收到.mmd>)
- [MV3-G05l 纯本地工具栏命令（多选 / 复制 / 提醒 / 协同 / 头脑风暴）](<./Phase1/post/Gate-05l-纯本地工具栏命令（多选-／-复制-／-提醒-／-协同-／-头脑风暴）.mmd>)

#### upload（6 Gate）

- [MV3-G01c 发送图片（多图合并一条）](<./Phase1/upload/Gate-01c-发送图片（多图合并一条）.mmd>)
- [MV3-G01d 发送文件](<./Phase1/upload/Gate-01d-发送文件.mmd>)
- [MV3-G01e 发送音频 / 视频](<./Phase1/upload/Gate-01e-发送音频-／-视频.mmd>)
- [MV3-G01g 发送模板图片](<./Phase1/upload/Gate-01g-发送模板图片.mmd>)
- [MV3-G02d 上传失败早退](<./Phase1/upload/Gate-02d-上传失败早退.mmd>)
- [MV3-G03c 图片预览内转发](<./Phase1/upload/Gate-03c-图片预览内转发.mmd>)

#### draft（1 Gate）

- [MV3-G02e 草稿保存与恢复](<./Phase1/draft/Gate-02e-草稿保存与恢复.mmd>)

#### schedule（5 Gate）

- [MV3-G04a 创建定时消息](<./Phase1/schedule/Gate-04a-创建定时消息.mmd>)
- [MV3-G04b 取消 / 删除定时消息](<./Phase1/schedule/Gate-04b-取消-／-删除定时消息.mmd>)
- [MV3-G04c 编辑定时消息（cancel + recreate）](<./Phase1/schedule/Gate-04c-编辑定时消息（cancel-+-recreate）.mmd>)
- [MV3-G04d 查询定时消息](<./Phase1/schedule/Gate-04d-查询定时消息.mmd>)
- [MV3-G04e 定时消息立即发送](<./Phase1/schedule/Gate-04e-定时消息立即发送.mmd>)

#### announcement（1 Gate）

- [MV3-G07b 保存群公告（新建 / 编辑）](<./Phase1/announcement/Gate-07b-保存群公告（新建-／-编辑）.mmd>)


### Phase2：读取、时间线与同步

#### read（5 Gate）

- [MV3-G06a 会话级已读](<./Phase2/read/Gate-06a-会话级已读.mmd>)
- [MV3-G06b 单条已读](<./Phase2/read/Gate-06b-单条已读.mmd>)
- [MV3-G06c 激活频道切换通知](<./Phase2/read/Gate-06c-激活频道切换通知.mmd>)
- [MV3-G06d readBits 位图与未读成员列表](<./Phase2/read/Gate-06d-readBits-位图与未读成员列表.mmd>)
- [MV3-G06e 未读总数投影与角标](<./Phase2/read/Gate-06e-未读总数投影与角标.mmd>)

#### timeline（10 Gate）

- [MV3-G11a 切群首屏](<./Phase2/timeline/Gate-11a-切群首屏.mmd>)
- [MV3-G11b 上拉更早历史](<./Phase2/timeline/Gate-11b-上拉更早历史.mmd>)
- [MV3-G11c 下拉加载更新消息](<./Phase2/timeline/Gate-11c-下拉加载更新消息.mmd>)
- [MV3-G11d 按 postId 定位（双向分页）](<./Phase2/timeline/Gate-11d-按-postId-定位（双向分页）.mmd>)
- [MV3-G11e 统一最新窗口读取](<./Phase2/timeline/Gate-11e-统一最新窗口读取.mmd>)
- [MV3-G11f 频道窗口切换](<./Phase2/timeline/Gate-11f-频道窗口切换.mmd>)
- [MV3-G11g 按 postId 批量取消息](<./Phase2/timeline/Gate-11g-按-postId-批量取消息.mmd>)
- [MV3-G11h 陌生消息窗口（getPostsAfter）](<./Phase2/timeline/Gate-11h-陌生消息窗口（getPostsAfter）.mmd>)
- [MV3-G12a 回复列表](<./Phase2/timeline/Gate-12a-回复列表.mmd>)
- [MV3-G12b 回复分支](<./Phase2/timeline/Gate-12b-回复分支.mmd>)

#### sync（5 Gate）

- [MV3-G13a hello 全量会话列表](<./Phase2/sync/Gate-13a-hello-全量会话列表.mmd>)
- [MV3-G13b 陌生频道兜底加载](<./Phase2/sync/Gate-13b-陌生频道兜底加载.mmd>)
- [MV3-G13c 同步 / gap 补偿](<./Phase2/sync/Gate-13c-同步-／-gap-补偿.mmd>)
- [MV3-G13d 子话题查询](<./Phase2/sync/Gate-13d-子话题查询.mmd>)
- [MV3-G13e 账号 Runtime 与 Renderer 释放](<./Phase2/sync/Gate-13e-账号-Runtime-与-Renderer-释放.mmd>)


### Phase3：频道与成员

#### channel（10 Gate）

- [MV3-G08b 创建话题](<./Phase3/channel/Gate-08b-创建话题.mmd>)
- [MV3-G08c 关闭群](<./Phase3/channel/Gate-08c-关闭群.mmd>)
- [MV3-G08d 退出群](<./Phase3/channel/Gate-08d-退出群.mmd>)
- [MV3-G10a 改群名](<./Phase3/channel/Gate-10a-改群名.mmd>)
- [MV3-G10b 改群公告字段（channel.notice）](<./Phase3/channel/Gate-10b-改群公告字段（channel.notice）.mmd>)
- [MV3-G10c 改群简介](<./Phase3/channel/Gate-10c-改群简介.mmd>)
- [MV3-G10d 会话置顶](<./Phase3/channel/Gate-10d-会话置顶.mmd>)
- [MV3-G10e 免打扰](<./Phase3/channel/Gate-10e-免打扰.mmd>)
- [MV3-G10f 会话导向](<./Phase3/channel/Gate-10f-会话导向.mmd>)
- [MV3-G10g 群权限（@ / 发公告 / 置顶）](<./Phase3/channel/Gate-10g-群权限（@-／-发公告-／-置顶）.mmd>)

#### channel_member（5 Gate）

- [MV3-G09a 拉人 / 踢人](<./Phase3/channel_member/Gate-09a-拉人-／-踢人.mmd>)
- [MV3-G09b 增加 / 移除管理员](<./Phase3/channel_member/Gate-09b-增加-／-移除管理员.mmd>)
- [MV3-G09c 改群成员昵称](<./Phase3/channel_member/Gate-09c-改群成员昵称.mmd>)
- [MV3-G09d 成员快照读取](<./Phase3/channel_member/Gate-09d-成员快照读取.mmd>)
- [MV3-G09e 成员在线状态](<./Phase3/channel_member/Gate-09e-成员在线状态.mmd>)


### Phase4：扩展消息操作与检索

#### pin（3 Gate）

- [MV3-G05f 消息置顶](<./Phase4/pin/Gate-05f-消息置顶.mmd>)
- [MV3-G05g 取消消息置顶](<./Phase4/pin/Gate-05g-取消消息置顶.mmd>)
- [MV3-G05h 加载置顶消息列表](<./Phase4/pin/Gate-05h-加载置顶消息列表.mmd>)

#### bookmark（3 Gate）

- [MV3-G05i 创建书签](<./Phase4/bookmark/Gate-05i-创建书签.mmd>)
- [MV3-G05j 删除书签](<./Phase4/bookmark/Gate-05j-删除书签.mmd>)
- [MV3-G05k 查询书签（分页）](<./Phase4/bookmark/Gate-05k-查询书签（分页）.mmd>)

#### search（3 Gate）

- [MV3-G14a 全局搜索（会话 / 联系人 / 消息三段）](<./Phase4/search/Gate-14a-全局搜索（会话-／-联系人-／-消息三段）.mmd>)
- [MV3-G14b 会话内分类搜索（5 tab）](<./Phase4/search/Gate-14b-会话内分类搜索（5-tab）.mmd>)
- [MV3-G14c 跨会话消息搜索并分组](<./Phase4/search/Gate-14c-跨会话消息搜索并分组.mmd>)

#### announcement（3 Gate）

- [MV3-G07a 查询群公告列表](<./Phase4/announcement/Gate-07a-查询群公告列表.mmd>)
- [MV3-G07c 删除群公告](<./Phase4/announcement/Gate-07c-删除群公告.mmd>)
- [MV3-G07d 群公告确认收到](<./Phase4/announcement/Gate-07d-群公告确认收到.mmd>)

### Phase5：接龙

- [接龙总览](<./Phase5/接龙总览.mmd>)

#### chain（6 Gate）

- [CHAIN-COM-G01 创建草稿并读取](<./Phase5/chain/Gate-COM-G01-创建草稿并读取.mmd>)
- [CHAIN-COM-G02 编辑并发布](<./Phase5/chain/Gate-COM-G02-编辑并发布.mmd>)
- [CHAIN-COM-G03 幂等追加与对账](<./Phase5/chain/Gate-COM-G03-幂等追加与对账.mmd>)
- [CHAIN-COM-G04 窗口展开](<./Phase5/chain/Gate-COM-G04-窗口展开.mmd>)
- [CHAIN-COM-G05 汇总计数](<./Phase5/chain/Gate-COM-G05-汇总计数.mmd>)
- [CHAIN-COM-G06 生命周期与已读](<./Phase5/chain/Gate-COM-G06-生命周期与已读.mmd>)

#### text_chain（2 Gate）

- [CHAIN-TXT-G01 文字参与](<./Phase5/text_chain/Gate-TXT-G01-文字参与.mmd>)
- [CHAIN-TXT-G02 文字窗口恢复](<./Phase5/text_chain/Gate-TXT-G02-文字窗口恢复.mmd>)

#### category_chain（3 Gate，已有源码实现；本轮运行态未验）

- [CHAIN-CAT-G01 分类模式](<./Phase5/category_chain/Gate-CAT-G01-分类模式.mmd>)
- [CHAIN-CAT-G02 分类选项](<./Phase5/category_chain/Gate-CAT-G02-分类选项.mmd>)
- [CHAIN-CAT-G03 分类统计](<./Phase5/category_chain/Gate-CAT-G03-分类统计.mmd>)

#### acceptance（1 Gate，本轮 NOT_VERIFIED）

- [CHAIN-ACC-G01 跨域验收](<./Phase5/acceptance/Gate-ACC-G01-跨域验收.mmd>)

## 图内统一边界

- HTTP 2xx 只表示 transport admission 或 query response；业务终态必须等待权威 WS/PortReply、
  `PersistOk`、读回和对应 `Effect::Emit {event,data}`。
- Native 的 `im:__bus__`、Flutter FFI batch、Web EventBatch 只搬运原始 `{event,data}`
  bytes；不在客户端 shell 复制 Helix 业务 reducer。
- `mock`、`helixTier`、`dual-user` 和当前 `partial/none` 只作为合同状态标注，
  不升级为真实 PC/移动/Web/Go/WS/Storage Green。
- 当前客户端合同和入口以 cses-client registry、phase manifest、各 Gate `contract.json`
  为真源；图中 Helix 路径用于描述目标接缝和现状边界，需运行态证据时回到对应 Gate 报告。
