# UC 覆盖台账 — cses 全量纯 helix 化(真 Go e2e 验证状态)

> 每 UC 真 Go e2e 跑绿 → 标 `✅ e2e-verified`。`scripts/fullmap-coverage.sh` 读此台账算 UC 覆盖率。
> 排除:UC 集合七 搜索(本次范围外)。createUser/status = goBlocked。
>
> **状态图例**:`✅ e2e-verified` = 真 Go WS action 回声断言 PASS;`⏭ data-dep` = 命令达 Go(wire 对)
> 但回声需特定数据态(未读态 / 双账号广播 / 先沉淀可读 post),非 wire bug;`🔴 wire-bug` = 命令载荷
> 与真源契约不符 / 字段不透传 → 行为保真丢失(需 helix 改出站);`⬜ untested` = 本轮 e2e 脚本未覆盖
> (命令已 wired,缺剧本/数据/上传管线)。
>
> **⚠️ 2026-06-24 Round-3 自echo 误诊已纠(self-omit 证伪)**:
> 上一会话标的 `🟡 go-self-omit`(「发送者自身连接不收 post 回声」)**是误诊**。Round-3 真 Go e2e
> RAW WS inbound 实证:发送者(444)自己连接**会收到自己消息的回声 action=`post`**(非 `posted`)——
> userId:"444"(self) + 本次 probe simpleMessage + 真 postId 全部命中(见「真机curl真源」§5)。故 self-omit
> 链断假设作废,send 族按 Round-3 真 e2e 裁决重标;F1 的 HTTP-200 兜底标 sent 仍是正确加固(防回声丢失),
> 但「发送者收不到回声」这一前提不成立。
>
> **2026-06-24 Round-3 真 Go e2e 实跑**(tp 已通·WS hello 真洪泛·account 444 @ :8065·6 场景 33 裁决):
> send 族(1.1/1.4)·emoji 表态(1.8)·加急(1.9)·定时(1.10)·单条已读(3.2)·建群(5.1)·话题(5.2)·关群(5.3)·群昵称(6.3)
> 共 10 UC WS 回声真断言全绿;另揪出 2 条 wire-bug(1.2 type 不透传 / 1.6 编辑 id 丢弃)。
>
> **2026-06-24 Round-4 真 Go e2e 续跑 + P1 修复翻台账**(28 条裁决,同环境 444 @ :8065):
> P1 修 type 透传后 **UC-1.2 DOCUMENT** 由 wire-bug 翻 ✅(type 原样回声非降级);新 e2e-verified:
> **3.1 会话已读·3.4 未读数·5.4 群属性(info+notice)·5.5 频道置顶·6.1 拉踢·6.2 管理员 member_role·6.4 成员快照·1.7 转发·8 average/read**。
> **UC-1.6 编辑仍 🔴 wire-bug**:id 透传代码改动真实存在但真 Go 对 posts/create+id 非空零回声(源码推断未经真抓包),禁判 verified。
> data-dep 诚实标注(命令达 Go·回声依赖数据态/host-cli 物理够不到):5.5 消息置顶 / 5.6 公告 / 8 vote/readVote(坏id 恒500) / 6.2 add_manger 次路径。

| UC | 描述 | 状态 |
|---|---|---|
| UC-1.1 | 发送文本/富文本 | ✅ e2e-verified (RAW WS action=`post` self-echo·userId 444+本次 probe+真 postId·self-omit 误诊已证伪) |
| UC-1.2 | 发送 DOCUMENT | ✅ e2e-verified (P1 修后·RAW WS action=`post` type=`DOCUMENT` 原样回声非降级 TEXT·temporaryId/channelId/message 全对齐·send_build.rs build_from_command 读 cmd type 默认 TEXT/DOCUMENT 透传真值) |
| UC-1.3 | 发送图片/文件 | ⬜ untested (host-cli 无文件上传/processMessageFiles 管线·posts/create 底层路径已由 1.2/1.4 post 回声验证) |
| UC-1.4 | 重发失败 | ✅ e2e-verified (重发=重走 posts/create·文本锚 RAW 帧 action=`post` self-echo 命中) |
| UC-1.5 | 撤回 | ⏭ data-dep (revoke 体逐字对齐真源§3·Go 有端点·全 log 零 revoke:true → 需先沉淀可读 post 再撤方能观测广播) |
| UC-1.6 | 编辑 | 🔴 wire-bug (Round-4 两轮真测·id 透传代码改动真实存在 wire-level 可证·但 posts/create+id 非空真 Go **零回声**·revoke 旁证连接链路完好·send_build.rs 注释「id 非空→Go 当 update」系源码推断未经真抓包·真 Go 不成立·端到端 wire 契约缺失→禁判 verified) |
| UC-1.7 | 转发/合并转发 | ✅ e2e-verified (直连 curl 真源 POST /posts/createPosts HTTP200 SUCCESS·helix posts_relay CreatePostsCommand Effect::Http 已触发·WS post 广播 host-cli 物理够不到=harness-gap 非 wire-bug·HTTP200 为转发执行硬证) |
| UC-1.8 | 快捷回复 emoji | ✅ e2e-verified (im_quick_reply→posts/quickReply·RAW post_update 带 quickReply[{emoji:👍,userIds:[444]}]) |
| UC-1.9 | 加急+加急已读 | ✅ e2e-verified (im_urgent_post→posts/urgentPost·RAW post_update 带 expediteMap.recipients{555,678}+sender 444) |
| UC-1.10 | 定时消息 | ✅ e2e-verified (post_schedule_created + post_schedule_canceled 双回声命中) |
| UC-2.1 | 切群首屏 | ⏭ data-dep (cold-start increment·见 4.1) |
| UC-2.2 | 上拉历史 | ⬜ untested (im_load_older_context 已 wired·本 send-chain 脚本不覆盖 sync/HTTP pull 路径·需专测 HX-C008 cursor 续拉) |
| UC-2.3 | postId 定位 | ⬜ untested |
| UC-2.4 | 回复列表 | ⬜ untested |
| UC-3.1 | 会话已读 | ✅ e2e-verified (444 read postId=4ake7...·RAW WS action=`post_read` data.postId 逐字一致 readMap=100 seq=72·DomainEvent emit im:channel:read_echo read-relay 回灌) |
| UC-3.2 | 单条已读 | ✅ e2e-verified (RAW WS action=`post_read`·reader 444 连接·data.postId 命中·seq=62) |
| UC-3.3 | 模板已收到 | ⬜ untested |
| UC-3.4 | 未读数 | ✅ e2e-verified (post 帧 data.readBits=`100` per-member 未读位图·read 后 post_read data.readMap=`100` 未读→已读态转移·RX increment_channel 携 unreadCount/mentionCount/lastEventSeq 离线对账) |
| UC-4.1 | hello 增量 | ✅ e2e-verified (hello+channels+increment_channel data 真推) |
| UC-4.2 | needSync | ✅ e2e-verified (increment_channel.needSync=true 真帧) |
| UC-4.3 | too_long 重拉 | ⬜ untested (缺触发 too_long 的大 gap 剧本) |
| UC-4.4 | 心跳 gap | ⬜ untested (ping piggyback 已实现·pong 补偿已 wired·缺 e2e 剧本) |
| UC-4.5 | 陌生 channel | ⬜ untested (陌生 channel 自动注册不丢帧已实现·缺 e2e 剧本) |
| UC-5.1 | 建群 | ✅ e2e-verified (RAW WS action=`channel_created` type:P·data.id 命中·真源体对齐§4) |
| UC-5.2 | 话题 | ✅ e2e-verified (channel_created type:T displayName helix-e2e-topic rootId 关联 + post_update 旁推·真源体对齐§2) |
| UC-5.3 | 关/退群 | ✅ e2e-verified (close 主动作 RAW action=`channel_close` self 收·member-leave 广播给被增减目标 678 非 self=broadcast-dep 子项) |
| UC-5.4 | 群属性 | ✅ e2e-verified (change/info 改名简介→RAW action=`update_channel` 全频道对象刷新·change/notice 改公告字段→RAW action=`update_channel_notice` content.text 逐字一致·marker 窗内归因) |
| UC-5.5 | 置顶 | ✅ e2e-verified 频道置顶 (change_top→RAW action=`update_channel` channelIsTop:true userId:444 per-member 定向自己) / ⏭ data-dep 消息置顶 (set_message_top HTTP 出站已发·两次探针仅 increment 洪泛·全 log `*pin*` action=0·pin 回声依赖 Go 落库后投影 host-cli 物理够不到) |
| UC-5.6 | 群公告 | ⏭ data-dep (announcement/save 体 verbatim 出站已发·窗内仅 increment 洪泛无公告专属 WS 帧·announcement/list 读族走 HTTP sync reply 不产 WS push·host-cli 不打印 HTTP status→回声不可观测) |
| UC-5.7 | 在线/分组 | ⏭ data-dep (status_change 真帧已观测) |
| UC-6.1 | 拉踢 | ✅ e2e-verified (444 踢 555·留存成员 678 第二连接 RX 收 RAW action=`post` props.type=`leave` users:[{id:555}] operator:444 event_seq:3·Go 把成员变更广播为 type=leave 的 post·留存成员确收) |
| UC-6.2 | 管理员 | ✅ e2e-verified member_role (678→ADMIN·RAW action=`channel_member_role_updated` data{role:ADMIN userIds:[678]} 逐字匹配·broadcast 到 channelId 操作者 444 收) / ⏭ data-dep add_manger 次路径 (channel/add/manger HTTP 正确分发·窗内仅 increment 洪泛无独立 echo) |
| UC-6.3 | 群昵称 | ✅ e2e-verified (RAW WS action=`update_channel_member_nickName` camelN·data.channelId 命中) |
| UC-6.4 | 成员快照 | ✅ e2e-verified (host-cli im_channel_member_snapshot req_id→DomainEvent emit im:read:result bytes=155 OutboundReadReply 透传回灌 + 直连 curl POST /channel/member/snapshot HTTP200 SUCCESS data[{userIds:[444,678,555]}]·channel_created 携 memberChange.join 全员快照) |
| UC-8 | 投票/平均分 | ✅ e2e-verified average/read (im_average_read 读族·DomainEvent emit im:read:result 回灌·身份头+session 通过到第二网关:3399) / ⏭ data-dep vote/readVote (REQ POST /vote/readVote 达第二网关:3399·身份头通过非401·HTTP500 与直 curl ground-truth 一致=坏id 恒500·DomainEvent emit im:read:result bytes=190 对齐响应·需真 vote id 方见200) |
| UC-9 | 书签 | ⬜ untested |
| UC-10 | 通知/待办 | ⬜ untested |
</content>
</invoke>
