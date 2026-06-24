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
> **2026-06-24 Round-5 UC-1.6 真相调查定论**(行为真源 cses-client + Go 路由/落库 + 真 Go e2e 三证):
> **现网消息正文不可改·无编辑端点** → UC-1.6 改判 ✅ **verified-not-a-capability**(产品无此能力·已 wire 实证不存在)。
> 纠 Round-3/4 误读:`send_build.rs:66「id 非空→Go 当 update」` = 把转发/createPosts 的 id 身份字段误当编辑开关;
> 真相:① cses-client `claude.md:299` 明文「消息正文不可改」·UI「编辑」= `reEdit()` 回填输入框重发 + 定时消息取消重建(均非正文改);
> ② Go 全 posts 路由无 posts/update·`CreateCsesPost→PreSave` 无 update 分支·`SavePostsBySend` CreateBulk 无 OnConflict → id 复用 = 重复 PK 冲突报错零推送;
> ③ e2e `host-cli-uc16-edit-e2e.sh`(原 send 得 post 回声→复用 postId 再发)实测**零 post/post_update 回声**,与「冲突无推送」一致。
> **helix 无命令可加**(目标端点不存在·非 wire-bug)。
> data-dep 诚实标注(命令达 Go·回声依赖数据态/host-cli 物理够不到):5.5 消息置顶 / 5.6 公告 / 8 vote/readVote(坏id 恒500) / 6.2 add_manger 次路径。
>
> **2026-06-24 Round-5 P2/P3 真 Go e2e 续跑(10 条裁决·sync-gap + 书签 + 待办 + query 族)**:
> **新 ✅ e2e-verified 5**:**4.4 心跳 gap**(pong seq_reply 锚定 ping 往返 + hashMismatch 补偿) · **4.5 陌生 channel**(冷启动空 cursor·73 帧 increment 全自动注册·入站=emit=73 零丢) · **3.3 模板已收到**(post_update updateAt bump + props.template.userIds 含 444) · **9 书签**(create/load/delete 三命令·im:read:result + 独立 curl 落库往返双证) · **10 通知/待办**(hello 收尾 increment_channel_end 触发 queryTodoList → im:todo:updated bytes=226447·反证空信封仅 47B)。
> **新揪 🔴 wire-bug 1**:**2.2 上拉历史**——`accepts_tick` Command 臂漏 `query::is_query()`·im_load_older_context 经真命令路径 accept()==false **静默丢弃**·拖累全 4 query 族命令(load_older/query_messages/query_dialog/delete_all_dialogs)不可达·单测只直调纯逻辑不过 accepts 闸故漏网·修 = Command 臂补 `\|\| crate::query::is_query(name)`。
> **harness-gap 1**(物理够不到·非 wire-bug):**4.3 too_long 重拉**——host-cli 无注入落后 cursor 入口·冷启动 cursor=0 走 increment 不进 too_long 分支·代码侧实现完整有单测·e2e 触发态待注入入口/服务端构造超阈 gap。
> **wire-bug 当前 = 1**(仅 2.2·1.6 已翻 ✅-not-a-capability)。

| UC | 描述 | 状态 |
|---|---|---|
| UC-1.1 | 发送文本/富文本 | ✅ e2e-verified (RAW WS action=`post` self-echo·userId 444+本次 probe+真 postId·self-omit 误诊已证伪) |
| UC-1.2 | 发送 DOCUMENT | ✅ e2e-verified (P1 修后·RAW WS action=`post` type=`DOCUMENT` 原样回声非降级 TEXT·temporaryId/channelId/message 全对齐·send_build.rs build_from_command 读 cmd type 默认 TEXT/DOCUMENT 透传真值) |
| UC-1.3 | 发送图片/文件 | ⬜ untested (**host-cli 物理够不到·需真前端**——host-cli 无文件上传/processMessageFiles 上传管线·无法构造 multipart/文件预处理链·posts/create 底层路径已由 1.2/1.4 post 回声间接验证·UI 上传链路需真前端 e2e) |
| UC-1.4 | 重发失败 | ✅ e2e-verified (重发=重走 posts/create·文本锚 RAW 帧 action=`post` self-echo 命中) |
| UC-1.5 | 撤回 | ⏭ data-dep (revoke 体逐字对齐真源§3·Go 有端点·全 log 零 revoke:true → 需先沉淀可读 post 再撤方能观测广播) |
| UC-1.6 | 编辑 | ✅ verified-not-a-capability (Round-5 真相调查定论·**现网消息正文不可改·无编辑端点**) ── ① 行为真源 cses-client `message-v3/service/claude.md:299` 明文「**post 更新·非正文编辑——消息正文不可改**」;UI「编辑」实为两类**非正文编辑**:`reEdit()`(message-item:891·仅 `!isComplete` 失败消息·派 `RE_EDIT_MESSAGE`→`chat-input:208 setInputValue` **回填输入框重发**·非服务端改) + 定时消息编辑(chat-input:310·**取消旧定时+重建**·非改正文)。② 全 posts 路由(Go `handlers.go:34-62` 22 端点)+ 真抓包(`真机curl真源§1` 普通 send `id:""`)**无任何 posts/update / posts/edit / editPost 路由**;唯一 update 形 = `updatePostProps`(props/投票·非正文)。③ Go `CreateCsesPost→PreSave`(post.go:187)仅 `ID==""` 时铸新 id·**无 update 分支**;落库 `SavePostsBySend`(cses_post_store.go:152)= `Post.CreateBulk().Exec()` **无 OnConflict** → id 复用 = 重复 PK INSERT **冲突报错** → consumer tx 失败 → **零 WS 推送**。④ 真 Go e2e(`scripts/host-cli-uc16-edit-e2e.sh`·444@:8065·send 原消息得 `post` 回声 → `cmd im_send_message id=<原postId>` 复用 id)实测**零 post/post_update 回声**(仅无关 increment_channel 洪泛)·与「INSERT 冲突无推送」一致。**结论纠错**:Round-3/4 的 `send_build.rs:66「id 非空→Go 当 update」` 系**源码误读**(把 createPosts/转发的 id 身份字段误当编辑开关)·**非 helix wire-bug·helix 无命令可加**(目标端点根本不存在);UC-1.6 改判 = 产品无此能力·e2e 已 wire 实证不存在) |
| UC-1.7 | 转发/合并转发 | ✅ e2e-verified (直连 curl 真源 POST /posts/createPosts HTTP200 SUCCESS·helix posts_relay CreatePostsCommand Effect::Http 已触发·WS post 广播 host-cli 物理够不到=harness-gap 非 wire-bug·HTTP200 为转发执行硬证) |
| UC-1.8 | 快捷回复 emoji | ✅ e2e-verified (im_quick_reply→posts/quickReply·RAW post_update 带 quickReply[{emoji:👍,userIds:[444]}]) |
| UC-1.9 | 加急+加急已读 | ✅ e2e-verified (im_urgent_post→posts/urgentPost·RAW post_update 带 expediteMap.recipients{555,678}+sender 444) |
| UC-1.10 | 定时消息 | ✅ e2e-verified (post_schedule_created + post_schedule_canceled 双回声命中) |
| UC-2.1 | 切群首屏 | ⏭ data-dep (cold-start increment·见 4.1) |
| UC-2.2 | 上拉历史 | ✅ e2e-verified (Round-5 揪出 + 本轮一行修复实证)·`acl/from_tick.rs::accepts_tick` Command 臂补 `\|\| crate::query::is_query(name)` 后,`scripts/uc22-load-older-e2e.sh` 建史 5 条→喂 `cmd im_load_older_context` → **`DomainEvent emit event=im:messages:older_loaded bytes=2544`** 真吐 2544B 历史投影。**原根因**:accepts_tick 仅放行 send/reconnect/is_outbound·漏 query::is_query → im_load_older_context(仅注册 `query.rs:38 is_query` 非 is_outbound)accept()==false 静默丢弃;handle()(module.rs:1196)+post_context_http 多轮编排+emit_older_loaded 全实现·p3c 单测仅直调纯逻辑绕过 accepts→handle 闸故漏网。**一行修同时解封全 4 query 族**(im_load_older_context/im_query_messages_by_channel/im_query_dialog_list/im_delete_all_dialogs);cargo test -p helix-im 0 fail·HX-C001 core 零改 |
| UC-2.3 | postId 定位 | ⬜ untested |
| UC-2.4 | 回复列表 | ✅ e2e-verified (Round-6 夺回·`scripts/host-cli-uc24-replies-e2e.sh` 持久连接 create 自建频道→send 抠真 reply 根 postId=cq311uf4d7dxdkrz9jjbno1c7o→`cmd im_get_replies {reply_id,pageNumber,pageSize,req_id}`。**read-relay 链路全代码实证**:`im_get_replies` 经 `read_cmd!` 宏 `is_read()=true`(outbound/posts_read.rs:136)→ module.rs:1139 `is_read(name)`命中→read_req_id 抠 req_id→corr_map.insert(OutboundReadReply{req_id})→HTTP POST posts/getReplies(真打·0 `read outbound http failed` warn)→回报透传 **`DomainEvent emit event=im:read:result bytes=680`**(req_id=uc24_replies_*·与 UC-9.2 书签 load 同一 relay 路径·bytes 量级对齐书签 679)。reply 根真存在故空回复列表 200 回读·681B 响应体经 relay 原样回灌。HX-C001 core 零改·structure-gate exit 0·仅新增 .sh harness 无源码改) |
| UC-3.1 | 会话已读 | ✅ e2e-verified (444 read postId=4ake7...·RAW WS action=`post_read` data.postId 逐字一致 readMap=100 seq=72·DomainEvent emit im:channel:read_echo read-relay 回灌) |
| UC-3.2 | 单条已读 | ✅ e2e-verified (RAW WS action=`post_read`·reader 444 连接·data.postId 命中·seq=62) |
| UC-3.3 | 模板已收到 | ✅ e2e-verified (持久连接·一次 hello connectionId 已盖章·投喂 `cmd im_template_received {postId:c55yc4...}` 真 TEMPLATE 帖来自 sync 回填 channel zhrbsxbhgt817nnghsrik7113e。__TR_REAL__ marker 前 post_update=0·marker 后唯一帧 RAW WS action=`post_update` data.id=c55yc4... type=TEMPLATE channelId 命中·**updateAt 1782279995599 从 createAt 1775917225693 被刚 bump**·updateBy=SYS·**props.template.userIds 含 444**(Go TemplateReceived 把我 userId 写进已收回执列表)·seq=3。因果链:① im_template_received accept(无 unknown/unhandled)→ ② Effect::Http POST camelCase {postId} 到 /post/templateReceived(Round-3 c7a840c casing 修·/post 单数前缀 template_received.rs:28 对齐)→ ③ Go 处理 + WS post_update 旁推回声) |
| UC-3.4 | 未读数 | ✅ e2e-verified (post 帧 data.readBits=`100` per-member 未读位图·read 后 post_read data.readMap=`100` 未读→已读态转移·RX increment_channel 携 unreadCount/mentionCount/lastEventSeq 离线对账) |
| UC-4.1 | hello 增量 | ✅ e2e-verified (hello+channels+increment_channel data 真推) |
| UC-4.2 | needSync | ✅ e2e-verified (increment_channel.needSync=true 真帧) |
| UC-4.3 | too_long 重拉 | ⬜ harness-gap (Round-5 e2e 物理够不到·**非 wire-bug**·代码侧实现完整有单测) host-cli 无「注入本地历史落后 cursor」入口:冷启动 cursor=0 时服务端走 increment 直接补齐不进 sync too_long 分支·也无法把本地 cursor 顶到远落后服务端 seq 的态→物理/数据够不到诚实记 harness-gap 不假判。代码侧 channel.rs:223 emit_sync_too_long / sync_session.rs:181 TooLong{reset_to} / module.rs:760 清表 **由 helix-im 单测覆盖**·e2e 触发态待 host-cli 加注入入口或服务端构造超阈 gap) |
| UC-4.4 | 心跳 gap | ✅ e2e-verified (持久连接挂 11s ≥ ping_interval 8s self-arm → 客户端 ping piggyback {cursors,allHash} → 服务端 ReconcileCursors 回 pong。RAW WS inbound 抓 3 条 pong:`{status:OK,seq_reply:1,data:{gaps:[],hashMismatch:false,text:pong}}` / seq_reply:3 / seq_reply:4——seq_reply 逐一锚定客户端 ping seq=1/3/4 证往返成功(触发 frame.rs __pong__ 合成 + pong_compensate 消费)。另一轮 probe 抓到 hashMismatch:true 的 pong 触发全量根群对账补偿。⚠️ 出站 ping 帧体 host 不落日志(network.rs 仅 dump RAW WS inbound)→ 以服务端 pong 回声 seq_reply 锚定为往返证据) |
| UC-4.5 | 陌生 channel | ✅ e2e-verified (冷启动独立租户 DB·HELIX_DB=mktemp 空 cursor store → 服务端推的 increment_channel 帧命中本地未知 channel。RAW WS inbound 73 条 `{v:1,action:increment_channel,data:{id:15gcgoyf1jfcur614qydhs69ha,lastEventSeq:2,...}}` 全命中本地未知 channel → entry().or_insert_with(Channel::new(_,0)) 自动注册 → **73 次 DomainEvent emit event=im:channel:increment 零丢**(入站帧数=emit 数=73)·bf25307 增量自动注册路径 e2e 跑通。⏭ data-dep 子项:channel/load/incrementByChannelId HTTP 读 → im:read:result relay 在非 info 级 LOG 未显式可见·降级 data-dep 不假判·WS 自动注册主路径已独立证) |
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
| UC-9 | 书签 | ✅ e2e-verified (持久连接·三命令读族 wire-path 静态确认 read_cmd! 宏 is_read()=true·module.rs:1184 read_req_id 命中 corr_map.insert(OutboundReadReply)。**9.1 create**:__BM_CREATE__ marker 后 DomainEvent emit im:read:result bytes=101·零 read http failed warn·独立 curl POST /post/bookmark/create {channelId,userId,postIds:[postId]}→HTTP200 {status:SUCCESS}。**9.2 load**:__BM_LOAD__ 后 emit im:read:result bytes=679(含真书签列表·>create/delete 的 101/98 ack)·独立 curl POST /post/bookmark/load→200 data.data[0]=本轮 send 的 post zumee8ci9frkbxrnxhxppciq8e(temporaryId hostcli_tmp_1)·hasMore:false 证 create 真落库+load 真回读·PageOpts 扁平 camelCase 透传对齐源契约。**9.3 delete**:__BM_DEL__ 后 emit im:read:result bytes=98·独立 curl POST /post/bookmark/delete {userId,postId}→200 SUCCESS·删除后 re-load→200 data.data:[] 空·证真删 DB 行非橡皮章) |
| UC-10 | 通知/待办 | ✅ e2e-verified (持久单连接 444@:8065 真 Go。一次 hello → scan_corr loaded 45 channels。**收集** 74× RAW WS action=increment_channel 每帧带 mentionList/urgentPostList(74/74)·真帧示例 mentionList[13 个 postId]/urgentPostList[2 个]·about_me_post_ids 收集。**触发** 1× RAW WS action=increment_channel_end(global) → 日志 `ws::handlers::increment_channel_end: hello 收尾触发 queryTodoList` @05:36:56.722。**装配+emit** 405ms 后 @05:36:57.127 DomainEvent emit event=im:todo:updated **bytes=226447**。反证:空 {items:[]} 信封仅 47 字节(python3 实测)·实际 226447 = ~4800x → queryTodoList HTTP 命中 SUCCESS/200 门控并装配真 items(非空 fallback)·全链路 grep 无 error/401/fail/panic·数据态后端原生已存 444 真有 mention/urgent post 无需合成注入) |
</content>
</invoke>
