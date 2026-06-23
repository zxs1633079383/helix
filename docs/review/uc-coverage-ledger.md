# UC 覆盖台账 — cses 全量纯 helix 化(真 Go e2e 验证状态)

> 每 UC 真 Go e2e 跑绿 → 标 `✅ e2e-verified`。`scripts/fullmap-coverage.sh` 读此台账算 UC 覆盖率。
> 排除:UC 集合七 搜索(本次范围外)。createUser/status = goBlocked。
>
> **状态图例**:`✅ e2e-verified` = 真 Go WS action 回声断言 PASS;`🟡 go-self-omit` = HTTP 200
> SUCCESS 已证(post 真落库) 但 Go 对**发送者自身连接不推 post 回声**(Mattermost self-omit),
> helix 当前 reconcile 依赖 WS `post` 回声(h1_corr 未注册) → sending 不转 sent(真 Go 阻断,见
> review finding F1);`⏭ data-dep` = 命令达 Go 但回声需特定数据态(如未读/双账号),非 wire bug;
> `⬜ untested` = 本轮未单独 e2e(命令已 wired,缺剧本/数据)。
>
> **2026-06-23 Phase 3 真 Go e2e 实跑**(account 444 @ :8065,4 独立会话复现):
> create/close/nickname/schedule/cancel WS 回声全绿;send post 回声 0/4 会话(self-omit 已证)。

| UC | 描述 | 状态 |
|---|---|---|
| UC-1.1 | 发送文本/富文本 | 🟡 go-self-omit (HTTP200 SUCCESS·post 回声 sender 不收) |
| UC-1.2 | 发送 DOCUMENT | ⬜ (走 posts/create·同 1.1 reconcile gap) |
| UC-1.3 | 发送图片/文件 | ⬜ (同 1.1) |
| UC-1.4 | 重发失败 | ⬜ |
| UC-1.5 | 撤回 | ⏭ data-dep (revoke 需 server postId·send 无回声链断) |
| UC-1.6 | 编辑 | ⬜ |
| UC-1.7 | 转发/合并转发 | ⬜ |
| UC-1.8 | 快捷回复 emoji | ⬜ |
| UC-1.9 | 加急+加急已读 | ⬜ |
| UC-1.10 | 定时消息 | ✅ e2e-verified (post_schedule_created + _canceled) |
| UC-2.1 | 切群首屏 | ⏭ data-dep (cold-start increment·见 4.1) |
| UC-2.2 | 上拉历史 | ⬜ (im_load_older_context 已 wired·缺剧本) |
| UC-2.3 | postId 定位 | ⬜ |
| UC-2.4 | 回复列表 | ⬜ |
| UC-3.1 | 会话已读 | ⏭ data-dep (read 达 Go·post_read 回声需未读态) |
| UC-3.2 | 单条已读 | ⏭ data-dep |
| UC-3.3 | 模板已收到 | ⬜ |
| UC-3.4 | 未读数 | ⏭ data-dep |
| UC-4.1 | hello 增量 | ✅ e2e-verified (hello+channels+increment_channel data 真推) |
| UC-4.2 | needSync | ✅ e2e-verified (increment_channel.needSync=true 真帧) |
| UC-4.3 | too_long 重拉 | ⬜ (缺触发 too_long 的大 gap 剧本) |
| UC-4.4 | 心跳 gap | ⬜ |
| UC-4.5 | 陌生 channel | ⬜ |
| UC-5.1 | 建群 | ✅ e2e-verified (channel_created·真源体对齐) |
| UC-5.2 | 话题 | ⏭ data-dep (makeTopic 需 send 的 postId·链断) |
| UC-5.3 | 关/退群 | ✅ e2e-verified (channel_close·自建频道安全销) |
| UC-5.4 | 群属性 | ⬜ (channel/change/info wired·缺 e2e) |
| UC-5.5 | 消息置顶 | ⬜ (post_pin handler 有·缺 e2e) |
| UC-5.6 | 群公告 | ⬜ (post/announcement/* wired·缺 e2e) |
| UC-5.7 | 在线/分组 | ⏭ data-dep (status_change 真帧已观测) |
| UC-6.1 | 拉踢 | ⏭ data-dep (member/change 达 Go·update 广播给被增减用户非 self) |
| UC-6.2 | 管理员 | ⬜ (channel_member_role_updated handler 有·缺 e2e) |
| UC-6.3 | 群昵称 | ✅ e2e-verified (update_channel_member_nickName camelN) |
| UC-6.4 | 成员快照 | ⏭ data-dep |
| UC-8 | 投票/平均分 | ⬜ (vote/vote + average/* wired·缺 e2e) |
| UC-9 | 书签 | ⬜ |
| UC-10 | 通知/待办 | ⬜ |
