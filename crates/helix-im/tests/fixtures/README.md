# Helix IM Canonical Fixture Gate 索引

本目录保存由 Helix `ScenarioRunner -> ImModule::handle -> Effect::Emit` 生成的 canonical
Frame。PC WDIO 与 Mobile GTest/QuickJS 可以镜像并消费这些文件，但不得把 consumer mirror
当成业务真源，也不得手工修改 JSON 来让消费端测试通过。

## 证据边界

- Fixture 证明 Helix producer 在确定性 sans-IO 剧本中产出了约定的 Frame。
- `provenance.json` 记录 fixture inventory、producer、schema、surface、raw/semantic hash
  与允许参数化的字段。
- Fixture/Mock 通过不等于 SQLite executor、真实 Go、WS、Tauri、DOM 或 Mobile 真机通过。
- 文件名中的 `.frame-v2.json` 是现有 artifact 命名，不代表内容一定是 schema 2；版本判断
  必须读取 JSON 的 `schemaVersion`。Timeline 可使用 schema 3，其他 Surface 仍可保持
  schema 2。

## 业务 Gate

> **生产证据边界（2026-07-28）**：仍在本目录的 G-10b/G-11a JSON 已改由
> `TimelineProductionRunner -> ImModule::handle -> Http/PortReply -> PersistOk -> readback ->
> Effect::Emit` 生成，并由各目录 `provenance.json` 固定 raw/semantic hash。它们可作为
> PC/Mobile 正式 Gate 的 Helix producer 输入；但仍只证明确定性 sans-IO 生产链，不替代
> 真实 Go、SQLite executor、Tauri/FFI、DOM 或真机证据。

| 目录 | Gate | 用户功能 | Fixture 主要阶段 |
|---|---|---|---|
| `g01-wdio/` | G-01 | 发送文本消息基线 | 建群、发送中、发送成功/失败、WS timeline 终态、左侧会话更新 |
| `g02-wdio/` | G-02 | 发送文档消息 | 文档卡片发送中/终态、失败、WS timeline 与 dialog 文档预览 |
| `g03a-wdio/` | G-03a | 失败消息重发 | 初次发送失败、重试发送中/成功、空 timeline、WS 对账与 dialog 更新 |
| `g04-wdio/` | G-04 | 在线撤回消息 | 可撤回、权威撤回终态、持久化恢复后的撤回状态 |
| `g04a-wdio/` | G-04a | 离线撤回重放 | 离线同步前状态、撤回事件重放、冷启动恢复 |
| `g05-wdio/` | G-05 | 转发/合并转发 | 源消息可转发、目标群转发消息、目标 dialog 预览 |
| `g06-wdio/` | G-06 | 快捷回复 emoji | 可 reaction、权威 reaction 终态、冷启动恢复 |
| `g07-wdio/` | G-07 | 消息加急与确认 | 可加急、加急生效、确认完成、参与人回执 pending/completed、成员列表 |
| `g08-wdio/` | G-08 | 创建定时消息 | 空 channel summary、定时状态生效、冷启动恢复 |
| `g09-wdio/` | G-09 | 取消定时消息 | 已定时、取消终态、冷启动恢复 |
| `g10-wdio/` | G-10 | 切群并加载最新消息首屏 | timeline ready、缓存降级、请求失败、竞态只接纳当前 generation、恢复 |
| `g10a-wdio/` | G-10a | 上拉加载更早历史 | older page 合并、历史耗尽、HTTP/持久化失败、竞态只接纳当前 generation |
| `g10b-wdio/` | G-10b | 下拉加载更新消息 | revision-1 latest Snapshot、定位 Delta、newer 尾插、耗尽边界、耗尽后晚到消息 |
| `g11a-wdio/` | G-11a | 未缓存消息定位与双向分页 | latest、两个并发 locate 乱序回放、当前 token 激活、旧 response 仅合并数据、定位后 older/newer |
| `g12-wdio/` | G-12 | 回复列表与话题分支 | thread snapshot、回复分支数据 |
| `g13-wdio/` | G-13 | 会话已读 | 初始未读 dialog、会话已读终态、冷启动恢复 |
| `g14-wdio/` | G-14 | 单条消息已读 | 可标记已读消息、发送者回执视图、pending/completed 回执、恢复 |
| `g14a-wdio/` | G-14a | 模板消息“已收到” | 可确认、权威已收到终态、冷启动恢复 |
| `g15a-wdio/` | G-15a | 创建群聊 | channel 创建、channel summary 与 member roster 恢复 |
| `g15-wdio/` | G-15 | 创建话题 | 话题线程创建、WS topic 事件、topic dialog、summary 恢复 |
| `g16-wdio/` | G-16 | 关闭群聊/退出群聊 | active、closed/left 终态及其恢复 |
| `g16b-wdio/` | G-16b | 成员退出广播 | active roster、成员退出后的 roster、恢复 |
| `g16c-wdio/` | G-16c | 修改群属性 | 群名称、简介、公告更新，观察者视角与恢复 |
| `gs1-wdio/` | G-S1 | hello 后全量/增量初始化 | dialog empty、loaded、recovery |
| `gs2-wdio/` | G-S2 | 离线 sync notify | dialog empty、sync 收敛、recovery |
| `gs3-wdio/` | G-S3 | pong gap 补偿 | dialog empty、gap 补偿收敛、recovery |
| `gs4-wdio/` | G-S4 | 陌生 channel 自动补全 | dialog empty、未知群补全后同步、recovery |

## 辅助 Fixture

| 目录 | 用途 |
|---|---|
| `f5/` | FFI egress parity 的 Golden 事件流，用于核对跨边界序列化结果 |
| `real_go_frames/` | 真实 Go 协议帧样本，覆盖 connect、increment channel、sync 与忽略事件 |
| `uc01-16-surface-runtime/` | SurfaceRuntime 原子 apply 合同，覆盖 snapshot/delta、重复、stale、gap、scope mismatch、action receipt 与持久失败 |

这些辅助目录不是独立业务 Gate，不计入上表业务分母。

## Producer 与更新入口

每个 `g*-wdio/` 或 `gs*-wdio/` 目录对应：

1. `../<gate>_wdio_fixture_contract_test.rs`：Fixture inventory、业务字段断言、Golden 与
   provenance 校验。
2. `../support/<gate>_scenario.rs` 或共享 ScenarioRunner：通过公开
   `ImModule::handle` 驱动真实 command/correlation/Persist barrier。
3. `provenance.json`：该 Gate 所有 artifact 的来源和 hash 清单。

普通测试只读比较 Golden。需要更新时，必须使用对应测试声明的 `UPDATE_<GATE>_WDIO_FIXTURES=1`
环境变量重新运行 producer test；不得直接编辑 Frame JSON。更新后还要同步 PC、Mobile
consumer mirror、loader、parity 测试与机器分母。

四个 Timeline V3 正式 Gate 的 L0 projector 合同分别由
`g10b_wdio_fixture_contract_test.rs`、
`g11a_wdio_fixture_contract_test.rs` 生成。
其 provenance 固定 Gate ID、测试类、producer revision、raw/semantic SHA-256，并把
operation kind、EntityKey、orderedKeys、range bounds、watermark、revision/baseRevision 与
windowToken 纳入语义 hash；允许参数化字段不包含这些业务合同字段。它们不得替代尚未落地的
production ScenarioRunner/provenance。

业务顺序、前置 Gate 和 Spec 入口见
[`specs/020-ifv2-three-repo-uc-control/plan-task-index.md`](../../../../specs/020-ifv2-three-repo-uc-control/plan-task-index.md)。
