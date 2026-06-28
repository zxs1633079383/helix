# UC 自动化测试编排谱（依赖序 + 数据流 · 面向 e2e 自动化）

> ⚠️ **SUPERSEDED（2026-06-28）**：本文（~40 UC 的 DAG 子集）已被 **`uc-command-set-spec.md`**（全量
> 121 outbound / 56 invoke / 27 emit / 19 WS + 内部通知映射 + DAG + 变量）取代；每 UC 四面机读契约见
> **`uc-test-contracts.md`**；后端补齐工单见 **`backend-gap-fills.md`**。本文 §0 DAG / §1 变量表已并入新文 §5/§6，
> 保留仅作历史参照。新增/改动请落新文，勿改本文。
>
> **用途**：给自动化测试搭建用的「执行顺序 + 上游产出→下游输入」真源。不是契约真源（契约见下方真源链），
> 这里只回答两件事：① **谁必须先跑、谁后跑**（防本末倒置：没群不能发消息、没消息不能撤回/已读/转发）；
> ② **每个 UC 产出哪些变量、被哪个下游 UC 当输入吃**（前置输出提取）。
> **范围**：排除 bot/agent 域 + Notification 域（用户裁决不测）。⛔ 后端阻塞 / 🌙 按需 单独标。
>
> **契约真源（只读，本文不重复字段）**：
> - HTTP/WS 全集：`csesapi-migration/full-map/partials/{1-posts,2-channel-sync,3-user-misc,5-ws-events}.md`
> - 投影 emit 契约：`projection-schema.md`（C2 冻结，21 emit）
> - outbound 请求体真抓包：`csesapi-migration/真机curl真源.md`
> - 穷举覆盖矩阵：loopforge `docs/uc-rollout/coverage-crossmap.md`（124 HTTP × 19 WS × UC）
> - 命令名以 host-cli registry 真名为准（`scripts/host-cli-FULL-uc-e2e.sh`）
>
> 更新 2026-06-26。

---

## 0. 依赖 DAG（一图看清先后）

```
[P0 连接] 4.1 hello ──产出 channel_ids[] / cursors ──┐
                                                      │（所有 UC 的前置：先有连接）
[P1 建容器] 5.1 create ──产出 channel_id(P) / member_user_ids[] ──┐
            11.1 team upsert ──产出 team_channel_id ──┐           │
                                                                  ▼
[P2 发消息] 1.1/1.2/1.4 send ──产出 post_server_id / temporary_id / event_seq ──┐
            1.10 schedule ──产出 schedule_post                                  │
                                                                                ▼
[P3 操作已发消息]（吃 post_server_id）
   1.5 撤回 · 1.8 快捷回复 · 1.9 加急 · 3.1/3.2 已读 · 3.3 模板已收到
   1.7 转发(吃 post_ids[] + 目标 channel_ids[]) ──产出 forwarded_post_ids[]
   2.4 回复列表(吃 reply_root_id ← 需先有回复链)
   5.2 makeTopic(吃 root post_id) ──产出 topic_channel_id(T)
                                                                                ▼
[P4 历史]（吃累积消息 + anchor post_id）  2.1 首屏 · 2.2 上拉 · 2.3 定位
[P5 频道/成员管理]（吃 channel_id）  5.4 改属性 · 5.5 置顶 · 6.1 拉踢 · 6.2 管理员 · 6.3 昵称 · 6.4 快照
[P6 杂项]  9 书签(吃 post_ids) · 10.1 待办(吃 post_ids) · 4.2 sync · 4.5 陌生channel · 4.4 心跳 · 8 投票/平均分
[P7 终结/运维]  12.1 health(任意时刻独立)
                                                                                ▼
[⚠️ 终结操作·每容器最后才跑]  5.3 关/退群(销毁 channel) · 11.2 退公司(销毁 team)
[L2 双账号广播·需第二连接]  US-17 · 6.1b · 5.3b · 6.2b
```

**5 条铁律（防本末倒置）**：
1. **4.1 连接最先**——无连接无一切。
2. **5.1 建群早于任何发消息**——message 必须有 channel_id 容器。
3. **send(1.1) 早于一切「对已发消息的操作」**——撤回/已读/快捷回复/加急/转发/回复/makeTopic/书签/待办/历史 全吃 post_server_id。
4. **2.4 回复列表前必须先造回复链**（父 post → 子回复 post → reply_root_id 才有值）。
5. **5.3 关群 / 11.2 退公司是销毁操作，必须放在该 channel/team 全部用例之后**（销毁后该容器所有下游失效）。

---

## 1. 变量提取登记表（上游产出 → 下游输入）★自动化最关键

> 自动化框架按此表把「上一步响应/帧里的字段」抽出来存成变量，喂给下一步命令体。键名 **snake_case**（铁律见 §3）。

| 变量 | 由哪个 UC 产出 | 从哪提取 | 被哪些下游 UC 当输入 |
|---|---|---|---|
| `connection_id` | 4.1 hello | WS hello 帧回注 | 全部（出站头/帧）|
| `existing_channel_ids[]` | 4.1 hello | `increment_channel` 帧 / `im:channels:loaded` | 2.x 历史 · 4.5 陌生channel 选未加载项 |
| `channel_id`(P) | **5.1 create** | `channel_created` 帧 data.id / `emit_channel_created` | 1.x 发消息 · 3.x 已读 · 5.4/5.5 属性 · 6.x 成员 · 5.3 关群 |
| `member_user_ids[]` | 5.1 create / 6.1 | create 入参 + `channel_member_update` | 1.9 加急 target · 6.2 管理员 · 6.3 昵称 |
| `post_server_id` | **1.1/1.2 send** | `post` 帧 data.id（tmp→server 重映射后）| 1.5 撤回 · 1.8 快捷回复 · 1.9 加急 · 3.2 已读 · 3.3 模板 · 9 书签 · 10.1 待办 · 5.2 makeTopic · 2.2/2.3 anchor |
| `temporary_id` | 1.1 send | 乐观上屏 `emit_post_sending` | 1.4 重发（复用同 temp_id 覆盖）|
| `event_seq` / cursor | 1.1 send + 4.1 | `post` 帧顶层 `event_seq` / `channel_event_cursor` | 4.2 sync notify(fromSeq) · 4.4 心跳(allHash/gaps)|
| `reply_root_id` | 回复链中的父 post | 子回复 post 的 reply_root_id | 2.4 getReplies/getReplyBranch |
| `forwarded_post_ids[]` | 1.7 createPosts | 各目标 channel 的 `post` 帧 | （后续对转发件的操作）|
| `topic_channel_id`(T) | 5.2 makeTopic | `channel_created`(type=T) | 话题内发消息/历史 |
| `team_channel_id` | 11.1 team upsert | `channel_created`/`emit_channel_update` | 11.2 退公司 |
| `req_id` | 调用方自生成 | — | **读族命令必带**（2.4/5.6/6.4/8/9/4.5 否则零 emit）|

---

## 2. 分阶段 UC × HTTP × WS 编排表

> 列：UC · 触发命令(host-cli)→outbound HTTP · 期望 WS/emit · 消费(输入) · 产出(给下游) · 序注。
> 状态：✅可测 / 🟡需双连接子项 / 🌙按需 / ⛔阻塞(列出但不铺)。

### P0 · 连接就绪根（无前置）
| UC | 命令→HTTP | 期望 WS/emit | 消费 | 产出 | 序注 |
|---|---|---|---|---|---|
| 4.1 hello 全量增量 | (WS hello 自动)/`channels/load/increment` | `increment_channel`(+_end) → `emit_channels_loaded`/`emit_channel_increment` | — | `channel_ids[]`/`cursors` | **最先** |

### P1 · 建容器（产出可发消息的 channel）
| 5.1 建群 | `im_create_channel`→`channel/create` | `channel_created`(P) → `emit_channel_created` | `member_user_ids[]`(fixture) | **`channel_id`(P)** / members | 发消息前必跑 |
| 11.1 维护公司大群 | `im_team_upsert`→`teams/upsert` | `channel_created`/`update_channel` | — | `team_channel_id` | teams 链入口 |

### P2 · 发消息（吃 channel_id → 产出 post_server_id）
| 1.1 发文本 | `im_send {channel_id,text}`→`posts/create` | `post` → `emit_post_received`(fat)+`emit_post_sending` | `channel_id` | **`post_server_id`/`temporary_id`/`event_seq`** | 操作类前置 |
| 1.2 发文档 | `im_send {channel_id,type=DOCUMENT}` | `post`(DOCUMENT) | `channel_id` | `post_server_id` | |
| 1.4 重发失败 | `im_send`(复用 `temporary_id`) | `emit_post_sending`→`emit_post_received` | `temporary_id`/`channel_id` | 覆盖同 post | 在 1.1 后 |
| 1.10 定时消息 | `posts/createSchedule`/`cancelSchedule` | `post_schedule_created`/`_canceled` | `channel_id` | schedule_post | |
| 1.3 发图片/文件 🌙 | (上传 java 接口·todo)+`posts/create` | `emit_post_received` | `channel_id`+file | — | 🌙 无上传管线·夜间 |

### P3 · 对已发消息操作（吃 post_server_id）
| 1.5 撤回 | `im_revoke {post_id}`→`posts/revoke` | `posts_update`(在线)/`post_update` → `emit_post_batch_updated`/`emit_post_deleted` | `post_server_id` | — | send 后 |
| 1.8 快捷回复 | `im_send_quick_reply {post_id,emoji}`→`posts/quickReply` | `post_update` → `emit_post_updated`(props.quickReply) | `post_server_id` | — | |
| 1.9 加急+加急已读 | `im_urgent_post {post_id,target_ids}`→`urgentPost`/`urgentConfirm` | `post_update` → `emit_post_updated`(expediteMap) | `post_server_id`/`target_user_ids[]` | — | |
| 3.2 单条已读 | `im_mark_read {channel_id,posts:[post_id]}`→`post/read` | `post_read` → `emit_post_read`(fat) 🟡L2 | `channel_id`/`post_server_id` | read_bits | |
| 3.1 会话已读 | `im_read_channel`→`channels/view {channels:[{id}]}` | `post_read`+`update_channel`(清badge) 🟡L2 | `channel_id` | read_bits | |
| 3.3 模板已收到 | `im_template_received {postId}`→`post/templateReceived` | `post_update` → `emit_post_updated`/read:result | `post_server_id` | — | **键名例外 camelCase** |
| 1.7 转发/合并 | `im_create_posts {posts,channelIds}`→`posts/createPosts` | 各目标 `post` → `emit_post_received` | `post_server_id[]`/目标`channel_ids[]` | `forwarded_post_ids[]` | |
| 2.4 一/二级回复 | `im_get_replies {reply_id,req_id}`→`getReplies`/`getReplyBranch` | `im:read:result`(读族) | `reply_root_id`/**`req_id`** | — | 先造回复链 |
| 5.2 创建话题 | `im_make_topic {rootId}`→`posts/makeTopic` | `channel_created`(T)+`post_update` | root `post_server_id` | **`topic_channel_id`(T)** | 吃一个 post |

### P4 · 历史（吃累积消息 + anchor）
| 2.1 切群首屏 | `im_query_messages_by_channel`(本地 Scan) | `emit_message_query_result`(透传) | `channel_id` | — | 需先有消息 |
| 2.2 上拉更早 | `im_load_older_context {channel_id,anchor_post_id,before:50}`→`posts/postContext` | `im:messages:older_loaded`(3键) | `channel_id`/anchor `post_server_id` | — | |
| 2.3 按 postId 定位 | `im_load_older_context {anchor_post_id}` | `emit_message_query_result`+高亮 | anchor `post_server_id` | — | |

### P5 · 频道/成员管理（吃 channel_id）
| 5.4 群属性修改 | `im_channel_change_info`/`_change_notice`/permission/orient... | `update_channel`/`update_channel_notice`/`change_channel_approval` | `channel_id` | — | 关群前 |
| 5.5 置顶 | `im_channel_change_top {channel_id,top}` | `update_channel`(channelIsTop) → `emit_channel_update`(thin) | `channel_id` | — | |
| 6.1 拉/踢人 | `im_channel_member_change {channelId,joinUsers/leaveUsers}` | `channel_member_update`/`post`(join/leave) | `channel_id`/`user_ids[]` | 更新 members | **键名例外 camelCase** |
| 6.2 设/撤管理员 | `im_channel_member_role {user_ids,role=ADMIN}`→`add/manger` | `channel_member_role_updated` 🟡L2 | `channel_id`/`user_ids[]` | — | |
| 6.3 改群昵称 | `nickname {channel_id}` | `update_channel_member_nickName` → `emit_member_nickname` | `channel_id`/`user_id` | — | |
| 6.4 成员快照 | `im_channel_member_snapshot {req_id}`/`channels/member/byIds` | `im:read:result`(读族) | `channel_id`/**`req_id`** | members | |

### P6 · 杂项
| 9 书签 | `im_bookmark_create/load/delete {post_ids,req_id}` | `im:read:result` | `post_server_id[]`/**`req_id`** | — | send 后 |
| 10.1 待办列表 | (hello 收尾自驱)`posts/queryTodoList {postIds}` | `im:todo:updated`{items} | `post_server_id[]` | — | |
| 4.2 按需 sync notify | `im_sync_channels`→`channel/sync/notify {cursors:[{channelId,fromSeq}]}` | `emit_channel_update_by_post`(thin)+`emit_post_received` | `channel_id`/`fromSeq(cursor)` | — | 吃 cursor |
| 4.5 陌生 channel 兜底 | `im_ensure_channel_loaded {channel_id}`(is_read=true,带`req_id`) | `im:read:result` | 未加载 `channel_id` | 自动注册 | |
| 4.4 心跳 gap 补偿 | (Rust ping/pong piggyback 自驱) | `pong {seq_reply,gaps,hashMismatch}` | `cursors`/`allHash` | — | 三面真绿(无③)|
| 8 投票/平均分 | `im_average_read {req_id}`/vote CRUD(:3399) | `emit_post_updated`(fat)/`im:read:result` | `channel_id`/`post_server_id`/**`req_id`** | — | 第二网关:3399 |

### P7 · 运维（独立）
| 12.1 健康探针 | `im_health`→`GET /health` | (无投影) | — | — | 任意时刻独立 |

### ⚠️ 终结操作（每容器全部用例之后才跑）
| 5.3 关闭/退出群 | `im_channel_close {channel_id}`→`channel/close` | `channel_close` → `emit_channel_closed` | `channel_id` | **销毁 channel** | **该 channel 最后** |
| 11.2 退出公司 | `im_team_quit`→`teams/member/quit`(DELETE)+WS`quit_company` | (member/channel 移除·🟡L2) | `team_channel_id` | **销毁 team** | **该 team 最后** |

### L2 · 双账号广播（需第二真实连接，L1 稳后专批）
| US-17 他人发消息收推送 | 连B发→连A收 | `emit_post_received` | 两连接 | — | 跨账号 |
| 6.1b 拉人后对端实时更新 | B被拉→A收 | `emit_channel_member_updated` | `channel_id` | — | |
| 5.3b member-leave 广播 | 第二连接收 | (member 广播) | `channel_id` | — | |
| 6.2b admin 广播到他人 | 第二连接收 role 变更 | `emit_channel_member_updated` | `channel_id` | — | |

### ⛔ 阻塞（列出但不铺，后端真阻塞/物理够不到）
- 1.6 编辑（无端点）· 5.6 公告（回声不可观测）· 5.7 在线状态/分组（statusCache 后端阻塞）· 7.x 搜索（store 空桩）· 4.3 too_long（harness 无 cursor 注入入口）· post_pin 消息置顶（data-dep）

---

## 3. 自动化执行铁律（决定真跑能否绿，照搬 [[helix-fast-e2e-harness]]）

1. **命令体键名 snake_case**：`post_id`/`channel_id`/`user_ids`/`target_ids`/`reply_id`/`post_ids`。
   **唯二例外 camelCase**：`im_template_received {postId}` · `im_channel_member_change {channelId,leaveUsers}`。
2. **读族命令必带 `req_id`**：否则 fire-and-forget 不注册 OutboundReadReply → **零 emit**。涉 2.4 / 5.6 / 6.4 / 8 / 9 / 4.5。
3. **变量提取走 §1 表**：每步把响应/帧字段抽成变量喂下一步；tmp→server id 重映射后才能当 anchor。
4. **依赖序 = §0 DAG**：连接→建群→发消息→操作→历史/管理→终结；销毁操作(5.3/11.2)永远垫底。
5. **matcher 直读文件 + `NO_COLOR=1`**：避免大文件 `set -o pipefail` 截断假 SKIP。
6. **单脚本双持久连接 + wait_for(~3min)**，不是 LLM-agent-workflow（30-60min）。脚本：`scripts/host-cli-FULL-uc-e2e.sh`。
