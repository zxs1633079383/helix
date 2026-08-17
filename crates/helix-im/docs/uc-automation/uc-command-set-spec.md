# UC 全量指令集总谱（入网 invoke × 出网 HTTP × WS 入站 × 内部通知 UI 层 · 面向自动化测试生成）

> **一句话**：helix-im 接管全部 IM 业务逻辑后，loopforge-tauri-im 退化为纯 UI 渲染壳。本文把
> 「一个 UC 从 UI 触发 → helix 入网命令 → 出网 HTTP → 后端 → WS 入站 → helix 投影 → 内部通知总线
> → UI data-* 渲染」这条全链路的**总和指令集**逐项梳理穷举，按 UC 维度 + 功能点 + 指令集合（含前后置
> 依赖）组织，**直接供自动化测试生成消费**。
>
> **配套文件**：
> - 每 UC 四面机读契约（`expect.{outbound,ws,projection,dom}` + preconditions + outputs）→ `uc-test-contracts.md`
> - 后端补齐工单（cses-im-server 3 处真改）→ `backend-gap-fills.md`
> - 本文 **supersedes** `uc-test-order-spec.md`（后者只覆盖 ~40 UC 的 DAG 子集，本文是全量 121 outbound / 56 invoke / 27 emit / 19 WS）
>
> **契约真源（只读，本文不重复字段，只索引）**：
> - 投影 emit 契约：`../projection-schema.md`（C2 冻结，22 工厂→21 channel）
> - outbound 请求体真抓包：`../csesapi-migration/真机curl真源.md`
> - HTTP/WS 全集：`../csesapi-migration/full-map/partials/{1,2,3,5}.md`
> - 穷举覆盖矩阵：loopforge `docs/uc-rollout/coverage-crossmap.md`（124 HTTP × 19 WS × UC）
> - 命令真名：loopforge `src-tauri/src/lib.rs` generate_handler! + helix outbound registry
>
> 更新 2026-06-28。

---

## 0. 三层指令拓扑（一图看清四类指令的接缝）

```
┌─ UI 层（Angular·纯渲染壳·零业务逻辑）────────────────────────────────────┐
│  app.component 模板事件 → ImStoreService → TauriBridgeService.invoke()      │
│  渲染读：listen("im:__bus__") → CoreEventManager.dispatch → DOM data-*      │
└───────────┬──────────────────────────────────────────▲────────────────────┘
       ① 入网 invoke（56 命令）                   ④ 内部通知（21 投影 channel）
            ▼                                          │
┌─ Tauri bridge（src-tauri/commands.rs·瘦转发）──────────────────────────────┐
│  #[tauri::command] → AppCommand → tick_tx 串行泵入 helix（camelCase→snake）  │
└───────────┬──────────────────────────────────────────▲────────────────────┘
            ▼                                          │ emit Effect::Emit
┌─ helix-im 引擎（业务逻辑唯一所在·sans-IO）─────────────────────────────────┐
│  outbound registry（121 命令）── build → Effect::Http ──┐                   │
│  ws/handlers（19 live action）── parse → projection ────┤                   │
│  acl/to_effect*（27 emit 工厂）── 投影 → im:__bus__ ────┘                   │
└───────────┬──────────────────────────────────────────▲────────────────────┘
       ② 出网 HTTP（124 endpoint）              ③ WS 入站（19 action）
            ▼                                          │
┌─ cses-im-server（Go 后端·:8066）─── 业务写 → hub.Broadcast WS 帧 ───────────┘
```

**四类指令 = 自动化测试要断言的四个面**：① 入网 invoke（驱动）· ② 出网 HTTP（facet outbound）·
③ WS 入站（facet ws）· ④ 内部通知投影（facet projection）→ UI data-*（facet dom）。

---

## 1. ① 入网 invoke 命令全集（UI → helix · 56 UI 驱动 / 121 引擎注册）

> **真名以 helix outbound registry 为准**（host-cli e2e 用引擎真名）；**loopforge invoke 名**是 UI 调的
> Tauri command 名（两者多数同名，少数壳侧聚合/改名，下表「壳 invoke」列标注）。
> 列：功能点 · 壳 invoke（UI 调）· helix outbound（引擎真名）· 出网 HTTP · 网关 · UC · 读/写。

### 1.A 消息收发（posts 写族）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | 网关 | UC | 类 |
|---|---|---|---|---|---|---|
| 发文本/文档 | `im_send` | (send_build) | `posts/create` | Im | 1.1/1.2/1.4 | 写 |
| 转发/合并 | `im_relay_messages` | `im_create_posts` | `posts/createPosts` | Im | 1.7 | 写 |
| 撤回 | `im_revoke` | `im_revoke` | `posts/revoke` | Im | 1.5 | 写 |
| 快捷回复 emoji | `im_send_quick_reply` | `im_quick_reply` | `posts/quickReply` | Im | 1.8 | 写 |
| 加急 | `im_urgent_post` | `im_urgent_post` | `posts/urgentPost` | Im | 1.9 | 写 |
| 加急确认 | `im_urgent_confirm` | `im_urgent_confirm` | `posts/urgentConfirm` | Im | 1.9 | 写 |
| 取消加急 | — | `im_urgent_cancel` | `posts/urgentCancel` | Im | 1.9 | 写 |
| 定时消息建 | `im_create_schedule` | `im_create_schedule` | `posts/createSchedule` | Im | 1.10 | 写 |
| 定时消息取消 | — | `im_cancel_schedule` | `posts/cancelSchedule` | Im | 1.10 | 写 |
| 模板已收到 | `im_template_received` | `im_template_received` | `post/templateReceived` | Im | 3.3 | 写 |
| 消息置顶/取消 | `im_post_pin` | `im_set_message_top`/`im_set_message_untop` | `channel/add/postPinned`·`remove/postPinned` | Im | 5.5b | 写 |
| 更新 post props | — | `im_update_post_props` | `posts/updatePostProps` | Im | 8.x | 写 |
| 消息审批 | — | `im_approval_post` | `post/approval/approval` | Im | ❓无 UC | 写 |

### 1.B 已读 / 历史 / 回复（posts 读族 · 必带 req_id）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 单条已读 | `im_mark_read` | `im_post_read` | `post/read` | 3.2 | 写* |
| 会话已读 | `im_read_channel` | `im_channels_view` | `channels/view` | 3.1 | 写* |
| 批量已读查 | — | `im_post_read_list` | `post/read/list` | 3.2 | 读 |
| 切群首屏（local-first） | `im_query_messages_by_channel` | 本地 Scan；coverage 非 Complete 时 `im_get_latest_post` | Complete: —；其余: `posts/getLatestPost` | 2.1 | 读 |
| 上拉更早 | `im_load_older_context` | `im_post_context` | `posts/postContext`×N | 2.2 | 读 |
| 按 postId 定位 | `im_load_older_context` | `im_get_posts`/`im_get_posts_after_index` | `posts/get`·`getPostsAfterIndex` | 2.3 | 读 |
| 一级回复 | `im_get_replies` | `im_get_replies` | `posts/getReplies` | 2.4 | 读 |
| 二级回复 | `im_get_reply_branch` | `im_get_reply_branch` | `posts/getReplyBranch` | 2.4 | 读 |
| 待办列表 | (hello 自驱) | `im_query_todo_list` | `posts/queryTodoList` | 10.1 | 读 |
| too_long 兜底首屏 | — | `im_top20`/`im_get_latest_post` | `posts/top20`·`getLatestPost` | 2.1 | 读 |
| 定时消息查 | — | `im_get_schedule` | `posts/getSchedule` | 1.10 | 读 |

> `*` 已读类是写族（落 read_bits）但回声经 `post_read` WS；自动化按写族断言（无 req_id）。

### 1.C 公告 / 书签（posts read_ext · 读族必带 req_id）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 公告保存 | `im_announcement_save` | `im_announcement_save` | `post/announcement/save` | 5.6w | 写 |
| 公告已读 | `im_announcement_read` | `im_announcement_read` | `post/announcement/read` | 5.6w | 写 |
| 公告删除 | `im_announcement_delete` | `im_announcement_delete` | `post/announcement/delete` | 5.6w | 写 |
| 公告列表 | `im_announcement_list` | `im_announcement_list` | `post/announcement/list` | 5.6r | 读 |
| 公告详情 | `im_announcement_detail` | `im_announcement_detail` | `post/announcement/detail` | 5.6r | 读 |
| 公告接收名单 | `im_announcement_accept_list` | `im_announcement_accept_list` | `post/announcement/acceptList` | 5.6r | 读 |
| 书签建 | `im_bookmark_create` | `im_bookmark_create` | `post/bookmark/create` | 9.x | 写 |
| 书签删 | `im_bookmark_delete` | `im_bookmark_delete` | `post/bookmark/delete` | 9.x | 写 |
| 书签列 | `im_bookmark_load` | `im_bookmark_load` | `post/bookmark/load` | 9.x | 读 |

### 1.D 频道管理（channel 写族）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 建群 | `im_create_channel` | `im_create_channel` | `channel/create` | 5.1 | 写 |
| 建群(指定owner) | — | `im_create_specify_owner` | `channel/createSpecifyOwner` | 5.1 | 写 |
| 建话题 | `im_make_topic` | `im_make_topic` | `posts/makeTopic` | 5.2 | 写 |
| 改群名 | `im_channel_change_display_name` | `im_channel_change_display_name` | `channel/change/displayName` | 5.4 | 写 |
| 改公告栏 | `im_channel_change_notice` | `im_channel_change_notice` | `channel/change/notice` | 5.4 | 写 |
| 改简介/info | — | `im_channel_change_info` | `channel/change/info` | 5.4 | 写 |
| 改权限 | — | `im_channel_change_permission` | `channel/change/permission` | 5.4 | 写 |
| 改头像 | — | `im_channel_change_picture` | `channel/change/picture` | 5.4 | 写 |
| 改来源 | — | `im_channel_change_source` | `channel/change/source` | 5.4 | 写 |
| 改朝向 | — | `im_channel_change_orient` | `channel/change/orient` | 5.4 | 写 |
| 改目的 | — | `im_channel_change_purpose` | `channel/change/purpose` | 5.4 | 写 |
| 开/关审批 | — | `im_channel_enable_approval` | `channels/enableApproval` | 5.4 | 写 |
| 频道置顶 | `im_channel_change_top` | `im_channel_change_top` | `channel/change/top` | 5.5 | 写 |
| 关闭/退群 | `im_channel_close` | `im_channel_close` | `channel/close` | 5.3 | 写 |
| 退群(leave) | — | `im_channel_leave` | `channel/member/leave` | 5.3 | 写 |

### 1.E 成员管理（channel member 写族）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 拉/踢人 | `im_channel_member_change` | `im_channel_member_change` | `channel/member/change` | 6.1 | 写 |
| 设/撤管理员 | `im_channel_set_manger` | `im_channel_add_manger`/`im_channel_remove_manger` | `channel/add/manger`·`remove/manger` | 6.2 | 写 |
| 改成员角色 | — | `im_channel_member_role` | `channel/member/change/role` | 6.2 | 写 |
| 成员免打扰 | — | `im_channel_member_notify` | `channel/member/change/notify` | 5.4 | 写 |
| 改群昵称 | `im_update_member_nickname` | `im_update_member_nickname` | `channel/member/change/nickname` | 6.3 | 写 |

### 1.F 频道/成员读族（必带 req_id）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 条件查频道 | `im_channel_query` | `im_channel_query` | `channel/query` | 5.8 | 读 |
| 在线状态 | `im_channel_online_status` | `im_channel_online_status` | `channel/onlineStatus` | 5.7 | 读 |
| 成员 byIds | `im_members_by_ids` | `im_channels_members_by_ids` | `channels/member/byIds` | 6.4 | 读 |
| 成员快照 | `im_member_snapshot` | `im_channel_member_snapshot` | `channel/member/snapshot` | 6.4 | 读 |
| 陌生 channel 兜底 | `im_ensure_channel_loaded` | `im_channel_load_increment_by_channel_id` | `channel/load/incrementByChannelId` | 4.5 | 读 |
| 管理员列表 | — | `im_channel_load_admin` | `channel/load/admin` | 6.2 | 读 |
| 公告栏回读 | — | `im_channel_load_notice` | `channel/load/notice` | 5.4 | 读 |
| 置顶消息列 | — | `im_channel_load_post_pinned` | `channel/load/postPinned` | 5.5 | 读 |

### 1.G 同步 / 连接 / 团队 / 杂项

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 冷启动会话列表 | `im_query_dialog_list` | (本地 Scan) | — | 4.1 | 读 |
| hello 全量增量 | (WS 自驱) | `channels/load/increment` | `channels/load/increment` | 4.1 | 读 |
| 按需 sync notify | `im_sync_channels` | (sync) | `channel/sync/notify` | 4.2 | 读 |
| 重连 | — | `im_reconnect` | (driver 连接生命周期) | — | 控制 |
| 维护公司大群 | `im_team_upsert` | `im_team_upsert` | `teams/upsert` | 11.1 | 写 |
| 加团队成员 | — | `im_team_member_add` | `teams/member/add` | 6.1 | 写 |
| 退出公司 | `im_team_quit` | `im_team_quit` | `teams/member/quit`(DELETE) | 11.2 | 写 |
| 全模块读 | `im_modules_get_all` | `im_get_all_modules` | `modules/getAll` | 10.3 | 读 |
| 健康探针 | `im_health` | `im_health`* | `GET /health` | 12.1 | 读 |
| 就绪 probe | `im_ready` | (引擎状态查询·无出网) | — | 4.1 | 内部 |

### 1.H 投票 / 平均分（第二网关 Default · :3399）

| 功能点 | 壳 invoke | helix outbound | 出网 HTTP | UC | 类 |
|---|---|---|---|---|---|
| 投票建/投/读/关/删/改截止/加成员 | `im_vote_create`/`_do`/`_read`/`_close`/`_delete`/`_update_finish_time`/`_add_members` | 同名 | `vote/{createVote,vote,readVote,closeVote,deleteVote,updateFinishTime,addMembersToVote}` | 8.x | 写+读 |
| 平均分建/参与/读/关/删/改截止/加成员 | `im_average_publish`/`_attend`/`_read`/`_close`/`_delete`/`_modify_cutoff`/`_add_member` | 同名 | `average/{publish,attend,read,close,delete,modifyCutoff,addMember}` | 8.x | 写+读 |

### 1.I 不在 UI 驱动集（引擎注册但 loopforge 无 invoke · 排除测试）

- **bot/agent 域**（36 命令 `outbound/bot_agent/*`）：🚫 用户裁决不测，无 UI。
- **search 域**（`im_search_post`/`_user`/`_channel`/`_do`）：✂️ 归属 java，ES 未迁。
- **开户/通知 域**（`im_list_users`/`im_get_user_statuses`/`im_load_send_notifications`/`im_load_target_notifications`）：✂️/❓ 深耦合原生或空骨架，无 UI invoke。
- **增量取代**（`im_get_updated_posts`）：被 channel_event v2 cursor sync（4.2）取代。

> **入网计数**：UI 驱动 invoke **56**（1.A–1.H 中标「壳 invoke」非「—」者）；引擎注册 outbound **121**
> （含 bot/agent 36 + vote/score 10 + 上述排除集）。自动化测试只生成 UI 驱动 56 的契约。

---

## 2. ② 出网 HTTP 全集（helix → cses-im-server · 124 endpoint）

> 逐个 path × UC × 覆盖判定的穷举矩阵见 loopforge `coverage-crossmap.md`（本文不重复）。汇总：

| 域 | endpoint 数 | ✅UI驱动 | 🚫bot | ✂️剔除 | ❓漏网 |
|---|---|---|---|---|---|
| A posts (partials/1) | 35 | 30 | 0 | 0 | 3（createMock/getUpdatedPosts/approval）|
| B channel+sync (partials/2) | 34 | 34 | 0 | 0 | 0 |
| C user-misc (partials/3) | 15 | 6 | 0 | 5（status/ids+search4）| 4（users/groups/loadSend/loadTarget）|
| D bot-agent (partials/4) | 40 | 0 | 37 | 3（demo）| 0 |
| **合计** | **124** | **70** | **37** | **8** | **7** |

> 70 ✅ 对应上面 56 UI invoke（多对一：部分 invoke 触发 ≥1 endpoint，如读族回读 + 翻页 ×N）。
> wire body 真源逐字段见 `真机curl真源.md`（6 命令真抓包）+ partials/1-3。

---

## 3. ③ WS 入站 action 全集（cses-im-server → helix · 19 live）

> handler 真源 `helix-im/src/ws/handlers/*`。② 投影列 = 该 action 触发哪个 emit。

| # | WS action | handler | → 投影 emit | 触发 UC |
|---|---|---|---|---|
| 1 | `post` | post.rs | `emit_post_received`(fat) | 1.1/1.2/1.4·6.1(leave post)·US-17 |
| 2 | `posts_update` | post_update.rs | `emit_post_batch_updated` | 1.5(撤回批) |
| 3 | `post_update` | post_update.rs | `emit_post_updated`(fat) | 1.5在线·1.8·1.9·3.3·5.6w·8.x |
| 4 | `post_read` | post_read.rs | `emit_post_read`(fat) | 3.1·3.2 |
| 5 | `post_pin` | post_pin.rs | `emit_post_updated` | 5.5b〔后端缺广播·见 backend-gap-fills〕 |
| 6 | `post_schedule_created` | post_schedule_created.rs | `emit_schedule_created` | 1.10 |
| 7 | `post_schedule_canceled` | post_schedule_canceled.rs | `emit_schedule_canceled` | 1.10 |
| 8 | `increment_channel` | increment_channel.rs | `emit_channel_increment` | 4.1·4.5 |
| 9 | `increment_channel_end` | increment_channel_end.rs | `emit_channel_update`(thin) | 4.1(就绪锚) |
| 10 | `channel_created` | channel_created.rs | `emit_channel_created` | 5.1·5.2·6.4 |
| 11 | `channel_member_update` | channel_member_update.rs | `emit_channel_member_updated` | 5.1·6.1 |
| 12 | `channel_member_role_updated` | channel_member_role_updated.rs | `emit_channel_member_updated`/`emit_channel_update` | 6.2〔manger add/remove 后端缺广播·见 backend-gap-fills〕 |
| 13 | `channel_close` | channel_close.rs | `emit_channel_closed` | 5.3 |
| 14 | `update_channel` | update_channel.rs | `emit_channel_update`(thin) | 5.4·5.5·3.1 |
| 15 | `update_channel_notice` | update_channel_notice.rs | `emit_channel_update`(thin) | 5.4 |
| 16 | `update_channel_member_nickName` | update_channel_member_nickname.rs | `emit_member_nickname` | 6.3 |
| 17 | `change_channel_approval` | change_channel_approval.rs | `emit_channel_update`(thin) | 5.4 |
| 18 | `quit_company` | quit_company.rs | (member/channel 移除) | 11.2 |
| 19 | `hello` | hello.rs | `emit_connection_established`+`emit_channels_loaded` | 4.1 |

> **dead/ignored/legacy（非业务·不测）**：`posted`/`post_urgent`/`post_edited`（P4 dead·合并进 post.rs）·
> `status_change`/`multiple_channels_viewed`/`typing`/`presence`（ignored no-op）· `__pong__`（心跳）·
> `__legacy_event__`（无 action 字段兜底）。

---

## 4. ④ 内部通知 UI 层映射表（emit 工厂 → im:__bus__ channel → UI 渲染目标 → data-*）★纯渲染壳的核心契约

> **这是「内部通知 ui 层」的一等公民总表**——helix 经单总线 `im:__bus__` emit 的 21 投影 channel，
> 前端 `CoreEventManager.dispatch` 逐 channel → 渲染目标（MessageRow/ChannelRow/MemberRow/读族 resolve）
> → DOM data-* 断言面。**fat=直渲零回读 / thin=低频回读刷 badge / 透传=resolve bridge**（分类真源 projection-schema §2）。
> **字段集冻结**见 `../projection-schema.md` §1（本表只给「channel → UI 目标 → data-* 断言键」映射，不重复字段集）。

| im:__bus__ channel | emit 工厂 | 类 | UI 渲染目标 | 关键 data-* 断言面 |
|---|---|---|---|---|
| `im:post:received` | emit_post_received | fat | 消息行新增 | `data-msg-id` `data-event-seq` `data-send-status` `data-type` `data-reactions` |
| `im:post:sending` | emit_post_sending | thin | 消息行乐观上屏 | `data-temporary-id` `data-send-status=sending` |
| `im:post:read` | emit_post_read | fat | 消息行已读刷新 | `data-read-bits` |
| `im:channel:read_echo` | emit_channel_read_echo | fat | 离线 read 回灌 | `data-read-bits` |
| `im:post:updated` | emit_post_updated | fat | 消息行元数据改 | `data-reactions` `data-urgent` `data-template-received` `data-pinned` `data-vote` `data-average` |
| `im:post:batch-updated` | emit_post_batch_updated | 透传 | 批量消息更新（撤回批） | `data-revoke` |
| `im:post:deleted` | emit_post_deleted | fat | 消息行移除/标删 | `data-revoke=1` / 行移除 |
| `im:channels:loaded` | emit_channels_loaded | 信号 | 冷启动列表刷新 | `data-ready` 链路 |
| `im:channel:increment` | emit_channel_increment | 透传 | 频道行快照 | `data-channel-id` `data-unread` `data-channel-display-name` |
| `im:channel:update` | emit_channel_update | thin | 频道行 badge（批结束） | `data-unread` `data-channel-top` |
| `im:channel:update-by-post` | emit_channel_update_by_post | thin | 频道行未读 +N | `data-unread` `data-mention` |
| `im:channel:created` | emit_channel_created | 透传 | 频道行新增 | `data-channel-id` `data-channel-type` |
| `im:channel:closed` | emit_channel_closed | — | 频道行移除 | 行移除（按 channelId） |
| `im:channel:member-updated` | emit_channel_member_updated | 透传 | 成员面板更新 | `data-members` `data-member-id` |
| `im:channel:members` | (render-ready 派生) | 派生 | 成员面板纯绑定 | `data-member-id` `data-admin` `data-nickname` |
| `im:channel:memberNickname` | emit_member_nickname | — | 成员昵称 | `data-nickname` |
| `im:channel:schedule-created` | emit_schedule_created | — | 频道定时标记 | `data-has-schedule-post` |
| `im:channel:schedule-canceled` | emit_schedule_canceled | — | 频道定时清除 | `data-has-schedule-post=null` |
| `im:messages:query_result` | emit_message_query_result | 透传 | 切群首屏渲染 | `data-msg-id`×N |
| `im:messages:older_loaded` | emit_older_loaded | 透传 | 上拉 prepend | `data-msg-id`(更早) |
| `im:channels:projection` | emit_dialog_list_result | fat | 会话列表渲染 | `data-channel-id`×N `data-unread` |
| `im:read:result` | emit_read_result/error | 透传 | 读族 bridge resolve | (按 endpoint：data-reply-id / data-bookmark-id / data-channel-id 列表) |
| `im:todo:updated` | emit_todo_updated | 装配 | 待办抽屉 | `data-todo-id` `data-todo-type` |
| `im:sync:too_long` | emit_sync_too_long | 控制 | 清 buffer + cursor 重置 | (无 DOM·内部 cursor) |
| `im:net:reconnect_requested` | emit_reconnect_requested | 控制 | driver 重连 | (无 DOM) |

> **27 emit 工厂 → 21 投影 channel + 6 非投影通道**（控制信号 reconnect / 读族 read_result×2 / 翻页 older_loaded /
> 待办 todo / 派生 members）。`data-system-notice` `data-highlighted` `data-user-id` 由 fat 字段在前端格式化派生（C013 纯绑定）。

---

## 5. 依赖 DAG（前后置 · 自动化拓扑排序真源）

```
[P0 连接] 4.1 hello ──产出 connection_id / existing_channel_ids[] / cursors ──┐（所有 UC 前置）
[P1 建容器] 5.1 create ──产出 channel_id(P) / member_user_ids[] ──┐
            11.1 team upsert ──产出 team_channel_id ──┐          │
                                                                 ▼
[P2 发消息] 1.1/1.2/1.4 send ──产出 post_server_id / temporary_id / event_seq ──┐
            1.10 schedule ──产出 schedule_post                                 │
                                                                               ▼
[P3 操作已发消息]（吃 post_server_id）
   1.5 撤回 · 1.8 快捷回复 · 1.9 加急 · 3.1/3.2 已读 · 3.3 模板已收到 · 5.5b 消息置顶
   1.7 转发(吃 post_ids[]+目标 channel_ids[]) ──产出 forwarded_post_ids[]
   2.4 回复列表(吃 reply_root_id ← 先有回复链)
   5.2 makeTopic(吃 root post_id) ──产出 topic_channel_id(T)
                                                                               ▼
[P4 历史]（吃累积消息+anchor post_id）  2.1 首屏 · 2.2 上拉 · 2.3 定位
[P5 频道/成员管理]（吃 channel_id）  5.4 改属性 · 5.5 置顶 · 6.1 拉踢 · 6.2 管理员 · 6.3 昵称 · 6.4 快照 · 5.7 在线 · 5.8 查频道 · 5.6 公告
[P6 杂项]  9 书签(吃 post_ids) · 10.1 待办(吃 post_ids) · 10.3 模块 · 4.2 sync · 4.5 陌生 · 4.4 心跳 · 8 投票/平均分
[P7 运维]  12.1 health(任意时刻独立)
                                                                               ▼
[⚠️ 终结·每容器最后]  5.3 关/退群(销毁 channel) · 11.2 退公司(销毁 team)
[L2 双账号广播·需第二连接]  US-17 · 6.1b · 5.3b · 6.2b
```

**5 条铁律**：① 4.1 连接最先 · ② 5.1 建群早于发消息 · ③ send(1.1) 早于一切对已发消息的操作 ·
④ 2.4 回复列表前先造回复链 · ⑤ 5.3 关群/11.2 退公司是销毁操作垫底。

---

## 6. 变量提取登记表（上游产出 → 下游输入 · 自动化最关键）

| 变量 | 由哪个 UC 产出 | 从哪提取 | 被哪些下游 UC 吃 |
|---|---|---|---|
| `connection_id` | 4.1 hello | WS hello 帧回注 / `im:connection:established` | 全部（出站头/帧）|
| `existing_channel_ids[]` | 4.1 hello | `increment_channel` 帧 / `im:channels:loaded` | 2.x 历史 · 4.5 选未加载项 |
| `channel_id`(P) | **5.1 create** | `channel_created` 帧 data.id / `im:channel:created` | 1.x · 3.x · 5.4/5.5 · 6.x · 5.3 |
| `member_user_ids[]` | 5.1 / 6.1 | create 入参 + `channel_member_update` | 1.9 target · 6.2 · 6.3 |
| `post_server_id` | **1.1/1.2 send** | `post` 帧 data.id（tmp→server 重映射后）| 1.5·1.8·1.9·3.2·3.3·5.5b·9·10.1·5.2·2.2/2.3 |
| `temporary_id` | 1.1 send | 乐观上屏 `im:post:sending` | 1.4 重发（复用覆盖）|
| `event_seq`/cursor | 1.1 + 4.1 | `post` 帧顶层 event_seq / channel_event_cursor | 4.2 sync(fromSeq) · 4.4 心跳 |
| `reply_root_id` | 回复链父 post | 子回复 post 的 reply_root_id | 2.4 getReplies/getReplyBranch |
| `forwarded_post_ids[]` | 1.7 createPosts | 各目标 channel `post` 帧 | 后续对转发件操作 |
| `topic_channel_id`(T) | 5.2 makeTopic | `channel_created`(type=T) | 话题内发消息/历史 |
| `team_channel_id` | 11.1 team upsert | `channel_created`/`emit_channel_update` | 11.2 退公司 |
| `req_id` | 调用方自生成 | — | **读族命令必带**（2.4/5.6r/6.4/8/9/4.5 否则零 emit）|

---

## 7. 自动化执行铁律（决定真跑能否绿）

1. **命令体键名 snake_case**：`post_id`/`channel_id`/`user_ids`/`target_ids`/`reply_id`/`post_ids`。
   **唯二例外 camelCase**：`im_template_received {postId}` · `im_channel_member_change {channelId,leaveUsers}`。
2. **读族命令必带 `req_id`**：否则 fire-and-forget 不注册 `OutboundReadReply` → **零 emit**。涉 2.4/5.6r/6.4/8/9/4.5。
3. **变量提取走 §6 表**：tmp→server id 重映射后才能当 anchor。
4. **依赖序 = §5 DAG**：销毁操作(5.3/11.2)永远垫底。
5. **四面契约只读·红转绿改实现不改 oracle**（loopforge C004）：live 失配优先改环境>机器件>契约。
6. **单脚本双持久连接 + wait_for(~3min)**，非 LLM-agent-workflow。脚本：helix `scripts/host-cli-FULL-uc-e2e.sh` / loopforge `scripts/run.sh`（seeded db）。

---

## 8. 与现有文档关系

| 文档 | 角色 |
|---|---|
| **本文 uc-command-set-spec.md** | 全量指令集总谱（入网×出网×WS×内部通知）+ DAG + 变量·**权威** |
| `uc-test-contracts.md` | 每 UC 四面机读 YAML 契约（测试生成直接吃）|
| `backend-gap-fills.md` | cses-im-server 3 处后端补齐工单 + 落地记录 |
| `uc-test-order-spec.md` | **superseded**（~40 UC DAG 子集，并入本文 §5/§6）|
| loopforge `coverage-crossmap.md` | 124 HTTP × 19 WS 穷举覆盖判定（本文 §2/§3 索引它）|
| `../projection-schema.md` | 21 投影 data 字段集冻结（本文 §4 索引它）|
| `../csesapi-migration/真机curl真源.md` | outbound wire body 真抓包（本文 §2 索引它）|
