# cses-client 全量纯 helix 化（100% 生产可用）— 设计 spec

> 2026-06-23 brainstorming 产出。**验收 = 100% 全量生产可用,非 demo / 非核心流程**。
> 硬契约 = `crates/helix-im/docs/csesapi-migration/full-map/partials/`（全部 UC + 全 HTTP + 全 WS）。

## 0. 目标与验收（DONE 定义）

**目标**:cses-client 退化为**纯 UI 渲染 + Tauri 薄壳**,**所有 IM 业务逻辑 100% 在 helix**(收发/解析/cursor/sync/SQLite/归一化/排序去重/上拉翻页/unread 计算/投影)。前端只:渲染 `im:__bus__` 投影 + emit 意图(invoke)+ 留纯 UI 态(scroll/选中/弹窗/表单)。

**DONE = 三个 100% 全绿 + 生产验证**(缺一不算完成):
1. **84 HTTP 全覆盖**:每接口 helix outbound 命令,wire 逐字对齐 `1-4--backend-http-*`（posts 35 + channel 34 + user-misc 15;bot-agent 不做,用户明确）。
2. **19 WS 全处理**:helix 解析→投影,对齐 `5--backend-ws-events`（post/posted/posts_update/post_read/post_update/channel_created/channel_member_update/role_updated/update_channel/update_channel_notice/change_channel_approval/increment_channel/increment_channel_end/post_schedule_created/post_schedule_canceled/add_followers/add_mentions/channels）。
3. **40 UC 全验证**:每 UC **真 Go e2e 跑绿**（invoke→helix→真 Go→WS 回声→helix 投影→前端渲染),非单测、非 demo。

**范围排除（用户 2026-06-23 拍板）**:
- **全局搜索（UC 集合七 searchPost/User/Channel）不在本次范围** → 从 40 UC 移除,剩 ~37 UC;对应 HTTP 搜索接口不计入。
- **createUser / status 接口 = goBlocked**（Go 物理缺 handler,user-misc 真阻塞）→ 标 goBlocked 如实列,不计入可覆盖 100%、不假装。

**可覆盖 100% 验收**:除上述排除外,**全部 ~37 UC + ~78 HTTP + 19 WS 做到 100% + 真 Go e2e 生产验证**。

## 1. 架构（边界已拍板）

```
helix (Rust sans-IO 核·拥有一切业务)        前端 (Angular·纯渲染)
────────────────────────────────────       ────────────────────────
SQLite 读写（store_* 全下放）               纯 UI 态:scroll / 选中 / 弹窗 / 表单输入
WS/HTTP I/O + 解析(parser)                  渲染 im:__bus__ 投影（零归一化/零计算）
cursor / sync / gate（peek→落库→commit）     emit 意图（invoke 语义命令）
归一化（normalizeMessageFromDB 下放）        ↑ 零业务逻辑、零本地 DB、零编排
排序/去重/合并/unread/badge 计算
上拉翻页（im_load_older_context）
84 HTTP + 19 WS 全部出站命令 + 投影
```

依赖铁律不变:helix-core 零运行时;helix-im 只依赖 core ports;前端不碰 wire/不碰 DB。

## 2. 完整性 checklist（100% 硬契约·逐条对账）

### 2.1 UC 全集（40 个,10 集合 → 单元）
| 集合（=迁移单元） | UC | 主前端文件 | helix 落点 |
|---|---|---|---|
| **一 消息收发** | UC-1.1~1.10（发送/文档/图片/重发/撤回/编辑/转发/快捷回复/加急/定时） | messageSend / message.service / ipcEventHandler | send_build + posts outbound + WS 回声投影 |
| **二 历史加载** | UC-2.1~2.4（首屏/上拉/定位/回复列表） | chatContentShared / Rust http.rs | im_query_messages_by_channel + **im_load_older_context（新）** |
| **三 已读未读** | UC-3.1~3.4（会话已读/单条/模板已收到/未读数） | message.service / messageDialog | post_read outbound + unread 计算下放 + 投影 |
| **四 离线同步** | UC-4.1~4.5（hello增量/needSync/too_long/心跳gap/陌生channel） | im-sync / message.service / Rust gate | SyncScheduler（已）+ WS increment 投影 |
| **五 群管理** | UC-5.1~5.7（建群/话题/关退/属性/置顶/公告/在线分组） | message.service | channel outbound（含 permission/messageTop/announcement 已补）|
| **六 成员管理** | UC-6.1~6.4（拉踢/管理员/昵称/快照） | message.service / channelMemberResolver | channel-member outbound |
| **七 搜索** | searchPost/User/Channel（api 网关） | message.service | outbound（gateway 维度,似 vote）|
| **八 互动卡片** | 投票/平均分（api 网关） | message-vote/average-content | **vote/score 第二网关:已 5 + 补 5（vote/closeVote/averageAttend/closeAverageScore/publishAverageScore）** |
| **九 书签** | bookmark CRUD（imHttp） | message.service | posts_read outbound（已）|
| **十 通知/待办** | UC-10.1~10.2 | messageNotice / Rust todo | outbound + 投影 |

### 2.2 HTTP 84 接口 / WS 19 事件
逐条 = `full-map/partials/1-4`（HTTP）+ `5`（WS）。Phase 3 对账脚本逐行核覆盖率。

## 3. 迁移单元（U=UC集合,边界=白名单,共 10 + 2 横切）

- **U1-U10 = 上表 10 个 UC 集合**(各自前端文件 + helix 落点)。
- **横切 X1 本地 DB 清除**:27 文件 store_* → helix 投影查询(贯穿 U1-U10)。
- **横切 X2 归一化清除**:normalizeMessageFromDB / formatMessageData → helix 吐归一化投影(贯穿)。
- 热文件(message.service 1267 / messageDialog 2891 / ipcEventHandler 1112)= 多 UC 共享 → **单 owner 串行改,其余并行**。

## 4. 执行模型（完整性驱动·loop-until-100%）

```
Phase 1 helix 命令全补（rust-dev·TDD·push main·CI 安全协议）:
   im_load_older_context（上拉翻页投影）+ vote/score 补 5 + 搜索 gateway + 通知/待办
   + dialog/unread 投影命令 + 归一化投影 + partials 24 个 partial 收口
Phase 2 cses 单元下放（cses-client-dev·并行有界 + 热文件串行）:
   U1-U10 各改纯渲染 + X1/X2 横切;每单元 tsc+jest 闸;红则自 revert/修
Phase 3 全量验收（真 Go e2e·full-map 三表对账）:
   40 UC 逐条真 Go e2e(444+64118eeb 自建群)+ 84 HTTP/19 WS 对账脚本
循环:三表对账未 100% → 下一轮 workflow（resumeFromRunId）接力,直到全绿
```

## 5. 验证 + 边界条件

- **完整性闸（新建 `scripts/fullmap-coverage.sh`）**:扫 full-map 三表 vs helix 已注册命令 + cses 已切方法 + e2e 已跑 UC → 输出覆盖率,**未 100% 退出码 1 = 未 DONE**。
- **每 UC 生产闸**:真 Go e2e 跑绿(非单测)→ 该 UC 可生产证据;e2e 用 444+64118eeb 自建群(已验证)+ 各 UC 专属断言(WS 回声 action + 投影字段)。
- **白名单零交集** + 热文件单 owner + 隔离分支 `feat/helix-takeover-integ` + git revert 兜底。
- **诚实红线**:Go 物理缺 → goBlocked 如实列;真 e2e 没跑通不判 shippable;不假装 wiring。
- **架构闸**:helix-core 零运行时 grep 空 + structure-gate + 命名陷阱保真(camelCase/createAt/unReadPostId)。

## 6. 风险与诚实 scope

- 84 HTTP + 19 WS + 40 UC + 15k 行迁移 = **大工程,多轮 workflow**。但**验收锚定 100%**:不是首轮交付即完,是 loop-until 三表全绿才算 DONE。每轮可验证、可中断、resumeFromRunId 续。
- 风险:热文件(messageDialog 2891)改动大 → 单 owner 串行 + 强 jest 回归兜。
- 回滚:全程隔离分支,main 不动;任一 UC 红 → revert 该 UC commit。
