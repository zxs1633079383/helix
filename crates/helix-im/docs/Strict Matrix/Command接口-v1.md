# Helix IM Command API v1

> 由 `contracts/strict-matrix.v1.json` 生成；禁止手工修改。
> contract epoch: 11; raw matrix sha256: `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`; semantic matrix sha256: `d0804a5f273a0c82c48a6ae2f16d37cc1bef973e895b036e26f6ef66b5480220`。
> raw hash 是正式 Helix commit blob、PC/Mobile pin 与生成物的唯一绑定值；semantic hash 仅用于诊断。

## 边界

- 本文件仅声明 Helix canonical command 与各 Gate 的输入、Effect、投影和合同状态；不表示 HTTP/WS/投影真实链已 Green。
- 同一 command 的多行 use 不能合并：每个 Gate 的 required/optional 字段和投影语义都必须独立保留。
- UI 只发 canonical command；host 只传输原始 JSON payload；alias、业务字段注入与 UI temporary id 均不属于 host。

## Canonical command uses

| Command | Gate / UC | Required | Optional | Effects | Projections | Lifecycle / Contract |
| --- | --- | --- | --- | --- | --- | --- |
| im_query_dialog_list | G-S1 / UC-4.1 hello 全量增量 | ∅ | limit | local_scan, projection | im:interaction:frame | active/declared |
| im_sync_channels | G-S2 / UC-4.2 sync notify | reason | channel_id, event_seq | sync_request, projection | im:interaction:frame | active/declared |
| im_sync_channels | G-S3 / UC-4.4 心跳 gap 补偿 | reason | channel_id, event_seq | sync_request, projection | im:interaction:frame | active/declared |
| im_sync_channels | G-04a / UC-1.5-offline 离线撤回重放 | reason | channel_id | sync_request, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_load_increment_by_channel_id | G-S4 / UC-4.5 陌生 channel 兜底 | channel_id, req_id | is_read | http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_send_message | G-01 / UC-1.1 发文本 | channel_id, text, type | props, replied, req_id | storage_upsert, http_write, timer, projection | im:interaction:frame | active/declared |
| im_send_message | G-02 / UC-1.2 发文档 | channel_id, type, props | text, replied, req_id | storage_upsert, http_write, timer, projection | im:interaction:frame | active/declared |
| im_send_message | G-03 / UC-1.3 发图片/文件/媒体 | channel_id, type, props | text | upload, http_write, projection | im:interaction:frame | active/declared |
| im_send_message | G-27 / US-17 他人发消息收推送 | channel_id, text, type | props | http_write, transport_broadcast, projection | im:interaction:frame | active/declared |
| im_retry_send | G-03a / UC-1.4 重发失败 | message_id | ∅ | storage_read, http_write, projection | im:interaction:frame | active/declared |
| im_revoke | G-04 / UC-1.5 撤回 | post_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_relay_messages | G-05 / UC-1.7 转发/合并转发 | post_ids, target_channel_ids, mode | ∅ | storage_read, http_write, projection | im:interaction:frame | active/declared |
| im_quick_reply | G-06 / UC-1.8 快捷回复 emoji | post_id, emoji | req_id | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_urgent_post | G-07 / UC-1.9 加急 + 确认 | post_id, target_ids | ∅ | http_write, projection | im:interaction:frame, im:interaction:frame | active/declared |
| im_urgent_confirm | G-07 / UC-1.9 加急 + 确认 | post_id | ∅ | http_write, projection | im:interaction:frame, im:interaction:frame | active/declared |
| im_create_schedule | G-08 / UC-1.10 定时消息 | channel_id, message, schedule_post_at | type, props | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_cancel_schedule | G-09 / UC-1.10-cancel 取消定时 | channel_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_query_messages_by_channel | G-10 / UC-2.1 切群首屏 | channel_id | limit | local_scan, http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_load_older_context | G-10a / UC-2.2 上拉更早历史 | channel_id, anchor_post_id | before | http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_get_posts | G-11 / UC-2.3 按 postId 定位 | post_ids, req_id | locate | http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_get_replies | G-12 / UC-2.4 回复列表/分支 | reply_id, req_id | channel_id, root_message_id, page_number, page_size, revoke | http_read, projection | im:interaction:frame | active/declared |
| im_get_reply_branch | G-12 / UC-2.4 回复列表/分支 | reply_first_level_id, req_id | channel_id, root_message_id, page_number, page_size | http_read, projection | im:interaction:frame | active/declared |
| im_channels_view | G-13 / UC-3.1 会话已读 | channels | ∅ | http_write, storage_upsert, projection | im:interaction:frame, im:interaction:frame | active/declared |
| im_post_read | G-14 / UC-3.2 单条已读 | channel_id, posts | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_template_received | G-14a / UC-3.3 模板已收到 | post_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_create_channel | G-15a / UC-5.1 创建群聊 | type, user_ids, display_name, force_create | ∅ | http_write, storage_upsert, projection | im:interaction:frame, im:interaction:frame | active/declared |
| im_make_topic | G-15 / UC-5.2 创建话题 | root_id, user_ids, req_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame, im:interaction:frame | active/declared |
| im_channel_close | G-16 / UC-5.3 关闭/退出群 | channel_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_leave | G-16 / UC-5.3 关闭/退出群 | channel_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_leave | G-16b / UC-5.3b member leave 广播 | channel_id | ∅ | http_write, transport_broadcast, projection | im:interaction:frame | active/declared |
| im_channel_change_display_name | G-16c / UC-5.4 群属性修改 | channel_id, display_name | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_channel_change_notice | G-16c / UC-5.4 群属性修改 | channel_id, notice | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_channel_change_info | G-16c / UC-5.4 群属性修改 | channel_id | purpose, header, expected_version | http_write, projection | im:interaction:frame | active/declared |
| im_channel_change_top | G-17 / UC-5.5 频道置顶 | channel_id, top | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_set_message_top | G-18 / UC-5.5b 消息置顶 | channel_id, post_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_set_message_untop | G-18 / UC-5.5b 消息置顶 | channel_id, post_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_announcement_list | G-18a / UC-5.6r/w 公告读写 | channel_id, req_id | ∅ | http_read, projection | im:interaction:frame | active/declared |
| im_announcement_save | G-18a / UC-5.6r/w 公告读写 | channel_id, post_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_announcement_read | G-18a / UC-5.6r/w 公告读写 | announcement_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_channel_online_status | G-18b / UC-5.7 在线状态 | channel_ids, req_id | ∅ | http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_query | G-19 / UC-5.8 条件查频道 | req_id, page, condition | ∅ | http_read, projection | im:interaction:frame | active/declared |
| im_channel_member_change | G-20 / UC-6.1 拉/踢人 | channel_id, join_users, leave_users | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_member_role | G-20a / UC-6.2 设/撤管理员 | channel_id, user_ids, role | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_update_member_nickname | G-20b / UC-6.3 改群昵称 | channel_id, user_id, nick_name | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_channel_member_snapshot | G-21 / UC-6.4 成员快照/全量修复 | channel_id, req_id | known_member_revision, known_member_digest | http_read, storage_replace, projection | im:interaction:frame | active/declared |
| im_channels_members_by_ids | G-21 / UC-6.4 成员快照/全量修复 | channel_ids, req_id | ∅ | http_read, storage_upsert, projection | im:interaction:frame | active/declared |
| im_search_post | G-22 / UC-7.x 搜索 | query, req_id | channel_id | http_read, projection | im:interaction:frame | excluded/blocked |
| im_vote_create | G-23 / UC-8.x-vote 投票 CRUD | channel_id, options | req_id | http_write, projection | im:interaction:frame | excluded/blocked |
| im_vote_read | G-23 / UC-8.x-vote 投票 CRUD | id, req_id | ∅ | http_read, projection | im:interaction:frame | excluded/blocked |
| im_average_publish | G-24 / UC-8.x-average 平均分 CRUD | channel_id | req_id | http_write, projection | im:interaction:frame | excluded/blocked |
| im_average_read | G-24 / UC-8.x-average 平均分 CRUD | id, req_id | ∅ | http_read, projection | im:interaction:frame | excluded/blocked |
| im_bookmark_create | G-25 / UC-9.x 书签 | post_id | ∅ | http_write, projection | im:interaction:frame | active/declared |
| im_bookmark_load | G-25 / UC-9.x 书签 | req_id | ∅ | http_read, projection | im:interaction:frame | active/declared |
| im_query_todo_list | G-26 / UC-10.1 待办列表 | req_id | channel_id | http_read, projection | im:interaction:frame | active/declared |
| im_load_target_notifications | G-26a / UC-10.2 系统通知 | req_id | channel_id | http_read, projection | im:interaction:frame | active/declared |
| im_get_all_modules | G-26b / UC-10.3 全模块读 | req_id | ∅ | http_read, projection | im:interaction:frame | active/declared |
| im_team_upsert | G-26c / UC-11.1 维护公司大群 | team_id, display_name | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |
| im_team_quit | G-26d / UC-11.2 退出公司 | team_id | ∅ | http_write, storage_upsert, projection | im:interaction:frame | active/declared |

## 禁止 alias

- `im_send`
- `im_send_quick_reply`
- `im_mark_read`
- `im_read_channel`
- `im_ensure_channel_loaded`
- `im_locate_post`
- `im_channel_create`
- `im_member_leave`
- `im_channel_settings`
- `im_post_pin`
- `im_announcement_write`
- `im_online_status`
- `im_members_by_ids`
- `im_list_candidate_users`
- `im_channel_admin_change`
- `im_member_nickname_change`
- `im_todo_query`
- `im_system_notice_query`
- `im_module_read`
- `im_company_group_upsert`
- `im_company_exit`
