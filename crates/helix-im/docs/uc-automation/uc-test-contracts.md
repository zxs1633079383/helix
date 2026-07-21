# UC 四面机读契约（每 UC 一块 YAML · 测试生成直接消费）

> **用途**：自动化测试生成器读本文每个 UC 的 YAML `contract` 块，生成四面断言用例。人读列在 YAML 上方一行。
> **配套**：拓扑/依赖/变量真源 → `uc-command-set-spec.md`；字段集冻结 → `../projection-schema.md`；wire body → `../csesapi-migration/真机curl真源.md`。
>
> **契约 schema（每块固定字段）**：
> ```
> uc / name / phase           # 标识 + 阶段（拓扑序见 spec §5）
> preconditions: [var...]     # 必须先存在的上游变量（拓扑前置）
> invoke: {command, args}     # ① 入网：壳调的命令 + 入参（$var 引用上游产出）
> expect:
>   outbound: {method,path,gateway,body_keys}   # ② 出网 HTTP（body_keys ref 真源）
>   ws: [{action, carries}]                      # ③ WS 入站帧
>   projection: [{channel, class}]               # ④ 内部通知投影（class: fat/thin/passthrough/signal/control）
>   dom: [{selector, assert}]                    # ⑤ UI data-* 断言（loopforge wdio 面）
> outputs: {var: source}      # 本 UC 产出给下游的变量 + 提取路径
> status: green|l2|backend-fix|on-demand         # 当前可测状态
> notes: ...                  # 序注/陷阱
> ```
> **状态图例**：`green`=四面可真跑全绿 · `l2`=主路径绿/广播面需第二连接 · `backend-fix`=依赖 backend-gap-fills 工单 · `on-demand`=🌙夜间。
>
> 更新 2026-06-28。

---

## P0 · 连接就绪根

### 4.1 hello 全量增量 — 所有 UC 的前置
```yaml
uc: "4.1"
name: hello 全量增量
phase: P0
preconditions: []
invoke: { command: "(WS hello 自动)/im_query_dialog_list", args: {} }
expect:
  outbound: { method: POST, path: /api/cses/channels/load/increment, gateway: Im, body_keys: [timestamp, cursors] }
  ws:
    - { action: hello, carries: [connectionId] }
    - { action: increment_channel, carries: [data.channelId, data.increment] }
    - { action: increment_channel_end, carries: [] }
  projection:
    - { channel: "im:connection:established", class: control }
    - { channel: "im:channel:increment", class: passthrough }
    - { channel: "im:channels:loaded", class: signal }
    - { channel: "im:channel:update", class: thin }
  dom:
    - { selector: "main", assert: { data-ready: "true" } }
    - { selector: "[data-channel-id]", assert: { data-channel-id: "<string>" } }
outputs:
  connection_id: "ws.hello.connectionId"
  existing_channel_ids: "ws.increment_channel[].data.channelId"
  cursors: "outbound.body.cursors"
status: green
notes: "就绪 probe = increment_channel_end 收齐 + HTTP inflight==0 + cursor 稳（最后确认步）"
```

---

## P1 · 建容器

### 5.1 创建群聊
```yaml
uc: "5.1"
name: 创建群聊
phase: P1
preconditions: [connection_id]
invoke: { command: im_create_channel, args: { type: "P", user_ids: ["$member_user_ids"], display_name: "t", force_create: false } }
expect:
  outbound: { method: POST, path: /api/cses/channel/create, gateway: Im, body_keys: [type, userIds, displayName, forceCreate] }
  ws:
    - { action: channel_created, carries: [data.id, data.channel] }
    - { action: channel_member_update, carries: [data.channelId] }
  projection:
    - { channel: "im:channel:created", class: passthrough }
    - { channel: "im:channel:member-updated", class: passthrough }
  dom:
    - { selector: "[data-channel-id]", assert: { data-channel-type: "P" } }
outputs:
  channel_id: "ws.channel_created.data.id"
  member_user_ids: "invoke.args.user_ids"
status: green
notes: "发消息前必跑·产出 channel_id 容器"
```

### 11.1 维护公司大群
```yaml
uc: "11.1"
name: 维护公司大群
phase: P1
preconditions: [connection_id]
invoke: { command: im_team_upsert, args: { team_id: "$team_id", display_name: "co" } }
expect:
  outbound: { method: POST, path: /api/cses/teams/upsert, gateway: Im, body_keys: [teamId, displayName] }
  ws: [ { action: channel_created, carries: [data.id] } ]
  projection: [ { channel: "im:channel:created", class: passthrough } ]
  dom: [ { selector: "[data-channel-id]", assert: { data-channel-type: "team" } } ]
outputs: { team_channel_id: "ws.channel_created.data.id" }
status: green
```

---

## P2 · 发消息

### 1.1 发文本
```yaml
uc: "1.1"
name: 发文本
phase: P2
preconditions: [channel_id]
invoke: { command: im_send, args: { channel_id: "$channel_id", text: "hello", type: "TEXT" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/create, gateway: Im, body_keys: [channelId, message, temporaryId, type, createAt] }
  ws: [ { action: post, carries: [data.id, event_seq] } ]
  projection:
    - { channel: "im:post:sending", class: thin }
    - { channel: "im:post:received", class: fat }
  dom:
    - { selector: "[data-temporary-id]", assert: { data-send-status: "sending" } }
    - { selector: "[data-msg-id]", assert: { data-send-status: "sent", data-event-seq: "<number>" } }
outputs:
  post_server_id: "ws.post.data.id"
  temporary_id: "invoke.args.temporaryId"
  event_seq: "ws.post.event_seq"
status: green
notes: "tmp→server 重映射后 data-msg-id 从 temporaryId 变 server_id；操作类前置"
```

### 1.2 发文档
```yaml
uc: "1.2"
name: 发文档
phase: P2
preconditions: [channel_id]
invoke: { command: im_send, args: { channel_id: "$channel_id", text: "doc", type: "DOCUMENT" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/create, gateway: Im, body_keys: [channelId, message, temporaryId, type] }
  ws: [ { action: post, carries: [data.id] } ]
  projection: [ { channel: "im:post:received", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-type: "DOCUMENT" } } ]
outputs: { post_server_id: "ws.post.data.id" }
status: green
```

### 1.4 重发失败
```yaml
uc: "1.4"
name: 重发失败
phase: P2
preconditions: [channel_id, temporary_id]
invoke: { command: im_send, args: { channel_id: "$channel_id", text: "retry", temporary_id: "$temporary_id" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/create, gateway: Im, body_keys: [channelId, message, temporaryId] }
  ws: [ { action: post, carries: [data.id] } ]
  projection:
    - { channel: "im:post:sending", class: thin }
    - { channel: "im:post:received", class: fat }
  dom: [ { selector: "[data-temporary-id]", assert: { data-send-status: "sent" } } ]
outputs: {}
status: green
notes: "复用同 temporary_id → upsert 覆盖；data-send-status failed→sending→sent"
```

### 1.10 定时消息
```yaml
uc: "1.10"
name: 定时消息
phase: P2
preconditions: [channel_id]
invoke: { command: im_create_schedule, args: { channel_id: "$channel_id", text: "later", schedule_at: "<ts>" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/createSchedule, gateway: Im, body_keys: [channelId, message, scheduleAt] }
  ws: [ { action: post_schedule_created, carries: [data.channelId, data.hasSchedulePost] } ]
  projection: [ { channel: "im:channel:schedule-created", class: signal } ]
  dom: [ { selector: "[data-channel-id]", assert: { data-has-schedule-post: "true" } } ]
outputs: { schedule_post: "ws.post_schedule_created.data" }
status: green
notes: "取消走 im_cancel_schedule → post_schedule_canceled → data-has-schedule-post=null"
```

### 1.3 发图片/文件 🌙
```yaml
uc: "1.3"
name: 发图片/文件
phase: P2
preconditions: [channel_id]
invoke: { command: im_send, args: { channel_id: "$channel_id", type: "DOCUMENT", props: { file: "<uploaded>" } } }
expect:
  outbound: { method: POST, path: /api/cses/posts/create, gateway: Im, body_keys: [channelId, props] }
  ws: [ { action: post, carries: [data.id] } ]
  projection: [ { channel: "im:post:received", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-send-status: "sent" } } ]
outputs: {}
status: on-demand
notes: "上传接口在 java·无上传管线·真go夜间"
```

---

## P3 · 对已发消息操作

### 1.5 撤回
```yaml
uc: "1.5"
name: 撤回
phase: P3
preconditions: [post_server_id]
invoke: { command: im_revoke, args: { post_id: "$post_server_id" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/revoke, gateway: Im, body_keys: [postId] }
  ws:
    - { action: posts_update, carries: [data.posts] }      # 在线批
    - { action: post_update, carries: [data] }             # 在线单
  projection:
    - { channel: "im:post:batch-updated", class: passthrough }   # 在线
    - { channel: "im:post:deleted", class: fat }                 # 离线
  dom: [ { selector: "[data-msg-id]", assert: { data-revoke: "1" } } ]
outputs: {}
status: green
```

### 1.8 快捷回复 emoji
```yaml
uc: "1.8"
name: 快捷回复 emoji
phase: P3
preconditions: [post_server_id]
invoke: { command: im_send_quick_reply, args: { post_id: "$post_server_id", emoji: "👍", user_id: "$self" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/quickReply, gateway: Im, body_keys: [postId, emoji, userId] }
  ws: [ { action: post_update, carries: [data.quickReply] } ]
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-reactions: "<contains 👍>" } } ]
outputs: {}
status: green
notes: "parser 把顶层 quickReply 合并进 props.quickReply；fat reactions 预解"
```

### 1.9 加急 + 加急已读
```yaml
uc: "1.9"
name: 加急+加急已读
phase: P3
preconditions: [post_server_id, member_user_ids]
invoke: { command: im_urgent_post, args: { post_id: "$post_server_id", target_ids: ["$member_user_ids"] } }
expect:
  outbound: { method: POST, path: /api/cses/posts/urgentPost, gateway: Im, body_keys: [postId, targetIds] }
  ws: [ { action: post_update, carries: [data] } ]
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-urgent: "1" } } ]
outputs: {}
status: green
notes: "expediteMap 不吐投影（需扩 PostFields）·data-urgent 走 DB 回读 expedite_map；确认走 im_urgent_confirm"
```

### 3.2 单条已读
```yaml
uc: "3.2"
name: 单条已读
phase: P3
preconditions: [channel_id, post_server_id]
invoke: { command: im_mark_read, args: { channel_id: "$channel_id", posts: ["$post_server_id"] } }
expect:
  outbound: { method: POST, path: /api/cses/post/read, gateway: Im, body_keys: [channelId, posts] }
  ws: [ { action: post_read, carries: [data] } ]
  projection: [ { channel: "im:post:read", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-read-bits: "<bitmap>" } } ]
outputs: { read_bits: "projection.im:post:read.data.readBits" }
status: l2
notes: "③ data-read-bits 本端✅；②④ 自发已读回声需第二连接观测他人已读位"
```

### 3.1 会话已读
```yaml
uc: "3.1"
name: 会话已读
phase: P3
preconditions: [channel_id]
invoke: { command: im_read_channel, args: { channel_id: "$channel_id" } }
expect:
  outbound: { method: POST, path: /api/cses/channels/view, gateway: Im, body_keys: [channels] }
  ws:
    - { action: post_read, carries: [data] }
    - { action: update_channel, carries: [data.channelId] }   # 清 badge
  projection:
    - { channel: "im:post:read", class: fat }
    - { channel: "im:channel:update", class: thin }
  dom:
    - { selector: "[data-msg-id]", assert: { data-read-bits: "<bitmap>" } }
    - { selector: "[data-channel-id]", assert: { data-unread: "0" } }
outputs: {}
status: l2
```

### 3.3 模板已收到
```yaml
uc: "3.3"
name: 模板已收到
phase: P3
preconditions: [post_server_id]
invoke: { command: im_template_received, args: { postId: "$post_server_id" } }   # ⚠️ camelCase 例外
expect:
  outbound: { method: POST, path: /api/cses/post/templateReceived, gateway: Im, body_keys: [postId] }
  ws: [ { action: post_update, carries: [data] } ]
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-template-received: "1" } } ]
outputs: {}
status: green
notes: "键名例外 camelCase postId（单数 path）"
```

### 5.5b 消息置顶 — ⚠️ 后端缺广播
```yaml
uc: "5.5b"
name: 消息置顶
phase: P3
preconditions: [channel_id, post_server_id]
invoke: { command: im_post_pin, args: { channel_id: "$channel_id", post_id: "$post_server_id" } }
expect:
  outbound: { method: POST, path: /api/cses/channel/add/postPinned, gateway: Im, body_keys: [channelId, postId] }
  ws: [ { action: post_pin, carries: [postId, channelId, operation] } ]   # ← backend-gap-fills #1 加上
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-pinned: "1" } } ]
outputs: {}
status: backend-fix
notes: "① 出站全绿；②③④ 依赖 backend-gap-fills #1（cses-im-server 加 post_pin 广播帧）"
```

### 1.7 转发/合并
```yaml
uc: "1.7"
name: 转发/合并
phase: P3
preconditions: [post_server_id, channel_id]
invoke: { command: im_relay_messages, args: { posts: ["$post_server_id"], channel_ids: ["$channel_id"] } }
expect:
  outbound: { method: POST, path: /api/cses/posts/createPosts, gateway: Im, body_keys: [posts, channelIds] }
  ws: [ { action: post, carries: [data.id] } ]   # 每目标 channel
  projection: [ { channel: "im:post:received", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-channel-id: "<target>" } } ]
outputs: { forwarded_post_ids: "ws.post[].data.id" }
status: green
```

### 2.4 一/二级回复
```yaml
uc: "2.4"
name: 一/二级回复
phase: P3
preconditions: [reply_root_id]
invoke: { command: im_get_replies, args: { reply_id: "$reply_root_id", req_id: "$gen" } }   # 读族必带 req_id
expect:
  outbound: { method: POST, path: /api/cses/posts/getReplies, gateway: Im, body_keys: [postId, reqId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: [ { selector: "[data-reply-id]", assert: { data-reply-id: "<postId>" } } ]
outputs: {}
status: green
notes: "先造回复链；二级走 im_get_reply_branch → getReplyBranch"
```

### 5.2 创建话题
```yaml
uc: "5.2"
name: 创建话题
phase: P3
preconditions: [post_server_id]
invoke: { command: im_make_topic, args: { root_id: "$post_server_id", user_ids: [], picture: "" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/makeTopic, gateway: Im, body_keys: [rootId, users, picture] }
  ws: [ { action: channel_created, carries: [data.id] } ]
  projection: [ { channel: "im:channel:created", class: passthrough } ]
  dom: [ { selector: "[data-channel-id]", assert: { data-channel-type: "T" } } ]
outputs: { topic_channel_id: "ws.channel_created.data.id" }
status: green
```

---

## P4 · 历史

### 2.1 切群首屏
```yaml
uc: "2.1"
name: 切群首屏
phase: P4
preconditions: [channel_id]
invoke: { command: im_query_messages_by_channel, args: { channel_id: "$channel_id" } }
expect:
  outbound: { method: POST, path: /api/cses/posts/getLatestPost, gateway: Im, body_keys: [channelId, timestamp], when: "coverage=Partial|Miss|Unsupported" }
  ws: []
  projection:
    - { channel: "im:messages:query_result", class: passthrough }
    - { channel: "im:post:increment-failed", class: control, when: "remote failed" }
  dom: [ { selector: "[data-msg-id]", assert: {}, count: ">0" } ]
outputs: {}
status: green
notes: "local-first 条件出网：Complete 必须以 trace/HTTP 计数证明零 HTTP；Partial/Miss/Unsupported 才走 getLatestPost。远端失败不得以空 query_result 冒充成功"
```

### 2.2 上拉更早
```yaml
uc: "2.2"
name: 上拉更早历史
phase: P4
preconditions: [channel_id, post_server_id]
invoke: { command: im_load_older_context, args: { channel_id: "$channel_id", anchor_post_id: "$post_server_id", before: 50 } }
expect:
  outbound: { method: POST, path: /api/cses/posts/postContext, gateway: Im, body_keys: [postId, before] }   # ×N 轮
  ws: []
  projection: [ { channel: "im:messages:older_loaded", class: passthrough } ]
  dom: [ { selector: "[data-msg-id]", assert: {}, count: "+prepend" } ]
outputs: {}
status: green
notes: "①②③ 全绿·④N/A（无 Persist effect）·≤8 轮 moving anchor"
```

### 2.3 按 postId 定位
```yaml
uc: "2.3"
name: 按 postId 定位
phase: P4
preconditions: [post_server_id]
invoke: { command: im_load_older_context, args: { anchor_post_id: "$post_server_id" } }
expect:
  outbound: null   # L1 本地 Scan；L2 越界翻页走 posts/get·getPostsAfterIndex
  ws: []
  projection: [ { channel: "im:messages:query_result", class: passthrough } ]
  dom: [ { selector: "[data-msg-id][data-highlighted='true']", assert: { data-highlighted: "true" } } ]
outputs: {}
status: green
```

---

## P5 · 频道/成员管理

### 5.4 群属性修改（改群名）
```yaml
uc: "5.4"
name: 群属性修改
phase: P5
preconditions: [channel_id]
invoke: { command: im_channel_change_display_name, args: { channel_id: "$channel_id", display_name: "new" } }
expect:
  outbound: { method: POST, path: /api/cses/channel/change/displayName, gateway: Im, body_keys: [channelId, displayName] }
  ws: [ { action: update_channel, carries: [data.channelId] }, { action: post, carries: [data] } ]
  projection: [ { channel: "im:channel:update", class: thin }, { channel: "im:post:received", class: fat } ]
  dom: [ { selector: "[data-channel-id]", assert: { data-channel-display-name: "new" } } ]
outputs: {}
status: green
notes: "改名产系统 NOTICE post；改公告走 change/notice→update_channel_notice；审批走 enableApproval→change_channel_approval"
```

### 5.5 频道置顶
```yaml
uc: "5.5"
name: 频道置顶
phase: P5
preconditions: [channel_id]
invoke: { command: im_channel_change_top, args: { channel_id: "$channel_id", top: true } }
expect:
  outbound: { method: POST, path: /api/cses/channel/change/top, gateway: Im, body_keys: [channelId, top] }
  ws: [ { action: update_channel, carries: [data.channelId] } ]
  projection: [ { channel: "im:channel:update", class: thin } ]
  dom: [ { selector: "[data-channel-id]", assert: { data-channel-top: "1" } } ]
outputs: {}
status: green
```

### 6.1 拉/踢人
```yaml
uc: "6.1"
name: 拉/踢人
phase: P5
preconditions: [channel_id, member_user_ids]
invoke: { command: im_channel_member_change, args: { channelId: "$channel_id", joinUsers: ["$uid"], leaveUsers: [] } }   # ⚠️ camelCase 例外
expect:
  outbound: { method: POST, path: /api/cses/channel/member/change, gateway: Im, body_keys: [channelId, joinUsers, leaveUsers] }
  ws: [ { action: channel_member_update, carries: [data.channelId] }, { action: post, carries: [data] } ]
  projection:
    - { channel: "im:channel:member-updated", class: passthrough }
    - { channel: "im:channel:members", class: passthrough }   # render-ready 派生
  dom: [ { selector: "[data-member-id]", assert: {} }, { selector: "[data-members]", assert: {} } ]
outputs: {}
status: green
notes: "键名例外 camelCase；后端已广播 channel_member_update（确认非缺口·见 backend-gap-fills 修正表）"
```

### 6.2 设/撤管理员 — ⚠️ manger add/remove 后端缺广播
```yaml
uc: "6.2"
name: 设/撤管理员
phase: P5
preconditions: [channel_id, member_user_ids]
invoke: { command: im_channel_set_manger, args: { channel_id: "$channel_id", users: ["$uid"], set: true } }
expect:
  outbound: { method: POST, path: /api/cses/channel/add/manger, gateway: Im, body_keys: [channelId, users] }
  ws: [ { action: channel_member_role_updated, carries: [channelId, userIds, role] } ]   # ← backend-gap-fills #2 加上
  projection: [ { channel: "im:channel:member-updated", class: passthrough } ]
  dom: [ { selector: "[data-member-id]", assert: { data-admin: "1" } } ]
outputs: {}
status: backend-fix
notes: "① 出站全绿·data-admin 乐观刷✅；channel/member/change/role 已广播，但 add/remove/manger 缺 → backend-gap-fills #2"
```

### 6.3 改群昵称
```yaml
uc: "6.3"
name: 改群昵称
phase: P5
preconditions: [channel_id, member_user_ids]
invoke: { command: im_update_member_nickname, args: { channel_id: "$channel_id", user_id: "$uid", nick_name: "n" } }
expect:
  outbound: { method: POST, path: /api/cses/channel/member/change/nickname, gateway: Im, body_keys: [channelId, userId, nickName] }
  ws: [ { action: update_channel_member_nickName, carries: [data.channelId, data.userId, data.nickName] } ]
  projection: [ { channel: "im:channel:memberNickname", class: signal } ]
  dom: [ { selector: "[data-member-id]", assert: { data-nickname: "n" } } ]
outputs: {}
status: green
```

### 6.4 成员快照/全量
```yaml
uc: "6.4"
name: 成员快照/全量
phase: P5
preconditions: [channel_id]
invoke: { command: im_member_snapshot, args: { channel_id: "$channel_id", req_id: "$gen" } }   # 读族必带 req_id
expect:
  outbound: { method: POST, path: /api/cses/channel/member/snapshot, gateway: Im, body_keys: [channelId, reqId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: [ { selector: "[data-members]", assert: {} } ]
outputs: {}
status: green
notes: "全量走 im_members_by_ids→channels/member/byIds"
```

### 5.7 在线状态 — ⚠️ users/status/ids 后端缺接线
```yaml
uc: "5.7"
name: 在线状态
phase: P5
preconditions: [channel_id]
invoke: { command: im_channel_online_status, args: { channel_ids: ["$channel_id"], req_id: "$gen" } }
expect:
  outbound: { method: POST, path: /api/cses/channel/onlineStatus, gateway: Im, body_keys: [channelIds, reqId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: []   # 读族·前端从 body 抽在线渲染
outputs: {}
status: green
notes: "channel/onlineStatus ①②全绿(go-served)；users/status/ids 已实现可用(api/user.go:27→app/cses_user.go:32→presenceQuery().OnlineUserIds·WS presence ZSET 真源)·非 statusCache 阻塞"
```

### 5.8 条件查频道
```yaml
uc: "5.8"
name: 条件查频道
phase: P5
preconditions: [connection_id]
invoke: { command: im_channel_query, args: { req_id: "$gen", page: 0, condition: {} } }
expect:
  outbound: { method: POST, path: /api/cses/channel/query, gateway: Im, body_keys: [reqId, page, condition] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: [ { selector: "[data-channel-id]", assert: {} } ]
outputs: {}
status: green
```

### 5.6r 公告读族
```yaml
uc: "5.6r"
name: 公告读族 list/detail/acceptList
phase: P5
preconditions: [channel_id, post_server_id]
invoke: { command: im_announcement_list, args: { channel_id: "$channel_id", req_id: "$gen" } }
expect:
  outbound: { method: POST, path: /api/cses/post/announcement/list, gateway: Im, body_keys: [channelId, reqId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: []
outputs: {}
status: green
notes: "①②全绿 go-served·③④N/A；detail→detail{postIds}, acceptList→acceptList{postId}"
```

### 5.6w 公告写族
```yaml
uc: "5.6w"
name: 公告写族 save/read/delete
phase: P5
preconditions: [channel_id, post_server_id]
invoke: { command: im_announcement_save, args: { channel_id: "$channel_id", post_id: "$post_server_id" } }
expect:
  outbound: { method: POST, path: /api/cses/post/announcement/save, gateway: Im, body_keys: [channelId, postId] }
  ws: [ { action: post_update, carries: [data] } ]   # 后端已广播（确认非缺口）
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: {} } ]
outputs: {}
status: green
notes: "后端 save/read/delete 均链到 publishPostUpdate→post_update（确认非缺口·见 backend-gap-fills 修正表）；旧 ⛔backend-down 是环境噪音"
```

---

## P6 · 杂项

### 9.x 书签
```yaml
uc: "9.x"
name: 书签
phase: P6
preconditions: [post_server_id]
invoke: { command: im_bookmark_create, args: { post_id: "$post_server_id" } }
expect:
  outbound: { method: POST, path: /api/cses/post/bookmark/create, gateway: Im, body_keys: [postId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]   # load 走读族
  dom: [ { selector: "[data-bookmark-id]", assert: {} } ]
outputs: {}
status: green
notes: "load 必带 req_id；delete 走 bookmark/delete"
```

### 10.1 待办列表
```yaml
uc: "10.1"
name: 待办列表
phase: P6
preconditions: [post_server_id]
invoke: { command: "(hello 收尾自驱)", args: {} }
expect:
  outbound: { method: POST, path: /api/cses/posts/queryTodoList, gateway: Im, body_keys: [postIds] }
  ws: []
  projection: [ { channel: "im:todo:updated", class: signal } ]
  dom: [ { selector: "[data-todo-id]", assert: {} } ]
outputs: {}
status: green
notes: "内核自驱·无 req_id·独立 TodoQuery 关联"
```

### 10.3 全模块读
```yaml
uc: "10.3"
name: 全模块读
phase: P6
preconditions: [connection_id]
invoke: { command: im_modules_get_all, args: { req_id: "$gen" } }
expect:
  outbound: { method: POST, path: /api/cses/modules/getAll, gateway: Im, body_keys: [] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: []
outputs: {}
status: green
```

### 4.2 按需 sync notify
```yaml
uc: "4.2"
name: 按需 sync notify
phase: P6
preconditions: [channel_id, event_seq]
invoke: { command: im_sync_channels, args: {} }
expect:
  outbound: { method: POST, path: /api/cses/channel/sync/notify, gateway: Im, body_keys: [cursors] }
  ws: [ { action: post, carries: [data] } ]
  projection:
    - { channel: "im:channel:update-by-post", class: thin }
    - { channel: "im:post:received", class: fat }
  dom: [ { selector: "[data-channel-id]", assert: { data-unread: "<n>" } } ]
outputs: {}
status: green
notes: "响应 kind=too_long(gap>5000)→resetTo→清表重拉（4.3 路径·后端已实现）"
```

### 4.5 陌生 channel 兜底
```yaml
uc: "4.5"
name: 陌生 channel 兜底
phase: P6
preconditions: [connection_id]
invoke: { command: im_ensure_channel_loaded, args: { channel_id: "$unknown", req_id: "$gen", is_read: true } }
expect:
  outbound: { method: POST, path: /api/cses/channel/load/incrementByChannelId, gateway: Im, body_keys: [channelId] }
  ws: []
  projection: [ { channel: "im:read:result", class: passthrough } ]
  dom: []
outputs: {}
status: green
notes: "读路径·不落新行·cursor 不推进"
```

### 4.4 心跳 gap 补偿
```yaml
uc: "4.4"
name: 心跳 gap 补偿
phase: P6
preconditions: [cursors]
invoke: { command: "(Rust ping/pong piggyback 自驱)", args: {} }
expect:
  outbound: { method: POST, path: /api/cses/channel/sync/notify, gateway: Im, body_keys: [cursors, allHash] }
  ws: [ { action: __pong__, carries: [seq_reply, gaps, hashMismatch] } ]
  projection: []   # 补偿走 4.2 sync 投影
  dom: []   # ③ N/A（已移除该面）
outputs: {}
status: green
notes: "三面真绿（①②④·无③）"
```

### 8.x 投票/平均分
```yaml
uc: "8.x"
name: 投票/平均分 CRUD
phase: P6
preconditions: [channel_id, post_server_id]
invoke: { command: im_vote_create, args: { channel_id: "$channel_id", req_id: "$gen", options: [] } }
expect:
  outbound: { method: POST, path: /vote/createVote, gateway: Default, body_keys: [channelId, options] }   # :3399 第二网关
  ws: [ { action: post_update, carries: [data] } ]
  projection: [ { channel: "im:post:updated", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-vote: "<json>" } } ]
outputs: {}
status: green
notes: "读类(read)走 im:read:result 必带 req_id；平均分同构 average/* data-average"
```

### 10.2 系统通知 — ⬜ 唯一未铺
```yaml
uc: "10.2"
name: 系统通知
phase: P6
preconditions: [connection_id]
invoke: { command: "(WS 帧触发·无独立 HTTP)", args: {} }
expect:
  outbound: null
  ws: [ { action: post, carries: [data] } ]   # SYSTEM/SYSTEN 类型
  projection: [ { channel: "im:post:received", class: fat } ]
  dom: [ { selector: "[data-msg-id]", assert: { data-system-notice: "1" } } ]
outputs: {}
status: green
notes: "⬜ 待铺·systemNotice 由 fat type∈NOTICE_TYPES 派生"
```

---

## P7 · 运维

### 12.1 健康探针
```yaml
uc: "12.1"
name: 健康探针
phase: P7
preconditions: []
invoke: { command: im_health, args: {} }
expect:
  outbound: { method: GET, path: /api/cses/health, gateway: Im, body_keys: [] }
  ws: []
  projection: []
  dom: [ { selector: "main", assert: { data-health: "ok" } } ]
outputs: {}
status: green
notes: "任意时刻独立·corr_key=req_id"
```

---

## 终结操作（每容器全部用例之后才跑）

### 5.3 关闭/退出群
```yaml
uc: "5.3"
name: 关闭/退出群
phase: TEARDOWN
preconditions: [channel_id]
invoke: { command: im_channel_close, args: { channel_id: "$channel_id" } }
expect:
  outbound: { method: POST, path: /api/cses/channel/close, gateway: Im, body_keys: [channelId] }
  ws: [ { action: channel_close, carries: [data.channelId, data.deleteAt] } ]
  projection: [ { channel: "im:channel:closed", class: signal } ]
  dom: [ { selector: "[data-channel-id]", assert: {}, expect: "removed" } ]
outputs: { destroys: "channel_id" }
status: green
notes: "销毁 channel·该 channel 所有下游用例之后"
```

### 11.2 退出公司
```yaml
uc: "11.2"
name: 退出公司
phase: TEARDOWN
preconditions: [team_channel_id]
invoke: { command: im_team_quit, args: { team_id: "$team_id" } }
expect:
  outbound: { method: DELETE, path: /api/cses/teams/member/quit, gateway: Im, body_keys: [teamId] }
  ws: [ { action: quit_company, carries: [data] } ]
  projection: []   # member/channel 移除·L2
  dom: [ { selector: "[data-channel-id]", assert: {}, expect: "removed" } ]
outputs: { destroys: "team_channel_id" }
status: l2
notes: "① L1✅·②③④ L2 第二连接观测移除广播"
```

---

## L2 · 双账号广播（需第二真实连接·L1 稳后专批）

```yaml
- uc: "US-17"
  name: 他人发消息收推送
  phase: L2
  setup: { conn_a: UI主, conn_b: HTTP副 }
  trigger: { who: conn_b, command: im_send }
  expect: { observe: conn_a, projection: [ { channel: "im:post:received", class: fat } ], dom: [ { selector: "[data-msg-id]" } ] }
  status: l2
- uc: "6.1b"
  name: 拉人后对端实时更新
  phase: L2
  trigger: { who: conn_a, command: im_channel_member_change, args: { joinUsers: ["$b"] } }
  expect: { observe: conn_b, projection: [ { channel: "im:channel:member-updated" } ], dom: [ { selector: "[data-members]" } ] }
  status: l2
- uc: "5.3b"
  name: member-leave 广播
  phase: L2
  trigger: { who: conn_a, command: im_channel_member_change, args: { leaveUsers: ["$b"] } }
  expect: { observe: conn_b, projection: [ { channel: "im:channel:member-updated" } ] }
  status: l2
- uc: "6.2b"
  name: admin 广播到他人
  phase: L2
  trigger: { who: conn_a, command: im_channel_set_manger, args: { set: true } }
  expect: { observe: conn_b, ws: [ { action: channel_member_role_updated } ], projection: [ { channel: "im:channel:member-updated" } ] }
  status: l2
  notes: "依赖 backend-gap-fills #2（manger 广播）"
```

---

## 排除集（不生成测试 · 真源 coverage-crossmap §诚实结论）

- 🚫 bot/agent 域（37 HTTP + 1 WS 变体·用户裁决不测）
- ✂️ search 域（4 endpoint·归属 java）· users/status/ids（statusCache·改由 backend-gap-fills #3 接线后转 5.7 子项）· cross-repo demo（3·非业务）
- ❓ 真漏网（客户端无 invoke）：createMock / getUpdatedPosts(被 4.2 取代) / approval / users(开户) / groups / notification/loadSend|loadTarget
- ⛔ 物理够不到：1.6 编辑（无端点）
