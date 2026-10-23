# Helix IM

`helix-im` 是运行在 Helix Core 上的确定性 MessageV3 IM 业务模块，负责消息领域状态机、
持久化屏障、恢复与业务事件生成，不包含平台 UI 或具体 I/O 实现。

```toml
[dependencies]
helix-im = "=0.1.85"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 合并转发投影

`im_relay_messages` 经共享调用门面映射到 `im_create_posts`，提交
`post_ids,target_channel_ids,mode,command_id,req_id`；HTTP 请求只携带 ID 意图、模式和幂等键，
不读取本地源全文，也不接收客户端快照树。入口见 [`client_api/command.rs`](src/client_api/command.rs)
与 [`forward.rs`](src/forward.rs)。

历史、实时消息和会话 lastPost 共用 [`query/render_ready/forward.rs`](src/query/render_ready/forward.rs)
的有界 `props.forward` 卡片：`snapshotId,title,itemCount,preview`，preview 最多 3 项；
不递归输出 `forwardDetail`。详情通过 `im_forward_detail({root_post_id,path,offset,req_id})`
按页读取 `/posts/forwardDetail`，结果经 `im:read:result.data.body` 返回；
`im_cancel_forward_detail({req_id})` 取消模块等待并释放关联/计时器，不直接中断 HTTP socket。
边界与失败处理见 [`query/forward_detail.rs`](src/query/forward_detail.rs)。

定向回归入口：`cargo test -p helix-im --test forward_snapshot_test`、
`cargo test -p helix-im --lib query::render_ready::forward::tests`。
这里只说明现有实现与测试入口，未在文档日检中重跑；客户端消费情况须核对其实际依赖与运行版本。

新发送的多附件摘要将数量放在方括号内，以空格分隔，如 `[图片 3个] 你好`、`[视频 2个]`、`[媒体 2个] 你好`。单附件不显示数量，摘要总长仍为 50 字；历史保存的摘要不重写。

### simpleMessage 统一摘要（2026-09-10）

`src/message_summary.rs` 为摘要规则唯一入口：发送准备、公告发送、服务端入站落库、历史与实时末条、合并转发/逐条转发共享。多选转发为 `[聊天记录]`；媒体沿用 `[图片 3个] 文字` / `[视频 2个] 文字` / `[媒体 2个] 文字`；业务卡片标签和标题、语音、系统通知也在此生成。系统通知使用服务端姓名快照，不编码查看者专属的“您”。稀疏状态更新不制造摘要。接龙声明和条目投影携带 `simpleMessage`。客户端不得再重算。定向回归：`cargo test -p helix-im --lib message_summary`、`cargo test -p helix-im --test p1_send_build_test`。

频道末条补齐（2026-09-10）：`prepare_post` 同时处理对象/JSON 字符串形式的 `lastPost`。频道完整落库、PATCH、member 绝对态及 created/update 事件统一准备摘要；建群完成投影保留首条通知与末条时间。无末条的稀疏事件不造消息，不更改 @、加急、草稿字段。回归 `cargo test -p helix-im --lib channel_summary`。

### 本端消息完成关联

`im:post:sent.data.requestId` 可选，来自本端发送的 `req_id`，仅在权威 WS / sync 对账以及发送终态写入成功后释放。
`im:post:received.data.requestId` 可选，只在命中本端 pending forward 的请求及目标、原子提交和成员读回成功后释放；接收方、重复目标和 HTTP admission 不携带该成功关联。
Host 可按提交前登记的请求匹配最近转发目标；不得把 `im:post:batch-result` 的 accepted/pending 当作送达。字段不进入 Go Post、持久消息体或 core/driver 协议，旧消费者可忽略。

### 实时消息的 @ 受众

`im:post:received.data.mentions` 直接输出 authority 中的受众，空数组也显式保留；
不由客户端根据消息文本或当前群成员推断。实现见
[`event/post.rs`](src/event/post.rs) 的 `authority_projection`。
已有隔离 SQLite 回归 `mentions_survive_persist_and_received_for_footer_notifications`
位于 [`empty_json_delivery.rs`](../helix-driver-host/tests/empty_json_delivery.rs)，
核对持久化与 received 受众一致、空数组和重复回声不重发。本轮仅核对源码与断言，未重跑测试或验证真实 UI。

## 全局书签与本机历史

- `im_bookmark_load_global({page_number,page_size,req_id})` 独立于群内书签查询，
  调用 `post/bookmark/global`；认证用户/企业取运行时身份，分页为正整数且
  `page_size <= 100`、`page_number <= 1000000`。参数与 wire 真源见
  [`outbound/posts/read_ext.rs`](src/outbound/posts/read_ext.rs)，结果/失败回灌见
  [`port_reply/dispatch_read.rs`](src/port_reply/dispatch_read.rs)。
- `im_bookmark_count({req_id})` 返回当前认证用户/企业的可见书签总数，使用会话内缓存，
  不写 SQLite。首次读取、断连失效后的读取或五分钟过期后的读取通过 `post/bookmark/count`
  校准；并发等待合并为一次 HTTP，结果复用 `im:read:result`。
  书签创建/删除按服务端 `affectedCount` / `countDelta` 更新已知总数；缺字段、身份不符或
  不合法增量使缓存失效，不凭 `SUCCESS` 猜计数。与写入重叠的旧校准响应不覆盖新状态；
  校准失败返回错误并退避 30 秒，身份切换/stop 取消等待者。实现见
  [`bookmark_count.rs`](src/bookmark_count.rs)，回归入口为
  `cargo test -p helix-im --test read_relay_test bookmark_count`。
- `im_recent_history` 的 `kind` 支持 `search`、`forward`、`emoji-send`、`emoji-reaction`，
  `action` 支持 `read`、`record`、`remove`、`clear`、`import`；表情发送与回应历史分开保存。
  写请求回传 read 返回的 `scope`，身份由运行时生成；通过既有 `im_recent_history` 表持久化，
  写入成功后返回新状态。完整 payload 与校验见 [`recent_history.rs`](src/recent_history.rs)。

以上为源码合同核对，不代表远端接口或客户端 UI 已在本轮验收。

## 群成员稀疏更新与显式清空

[`channel/write/members.rs`](src/channel/write/members.rs) 按字段合并成员来源：当前 `members`
优先于角色摘要，角色摘要优先于历史 `memberChange.join`。缺省字段保留已有值；
`nickName: ""` 明确清空昵称，缺少或 null 昵称不生成清空。`boss[].option=leave` 只撤销
BOSS 角色，成员离场由 `memberChange.leave` 表达，不能从空角色摘要推断离场。

[`acl/to_effect_s1/storage.rs`](src/acl/to_effect_s1/storage.rs) 根据 `omitted_fields` 将成员
分为最多 8 个固定字段形状批次，通过 `exclude_from_update` 保留冲突行中的缺省字段；
默认值只用于新行。定向回归入口：
`cargo test -p helix-im --test channel_empty_semantics_regression_test`、
`cargo test -p helix-driver-host --test channel_empty_semantics_sqlite`。
上述测试入口已核对源码与 Cargo 注册，本次纯文档日检未执行 Rust 或真实业务链验收。

## 统一失败来源

[`failure_context.rs`](src/failure_context.rs) 在 Module 边界关联命令、HTTP 路径或 WS action、
阶段、`req_id` 与 `corr`；只保存来源元数据，已释放请求的最近来源最多保留 256 项，
stop/身份切换清空关联。成功事件跳过失败解析，失败保留机器原因码，按事件合同补充
错误文本或 `errorContext`；固定 `-failed` 事件不新增字段，只在已有 `message` 中附加
宿主注入的 `[trace_id=…]`，没有 TraceId 时保留其原始 bytes。

TraceId 来自 core 的当前 step 诊断，不由业务模块创建；HTTP/WS/业务错误回归入口为
`cargo test -p helix-im --test failure_context_test`。本次核对实现与测试入口，未重跑测试或验收远端失败链。

## 邀请与成员管理权限

[`query/render_ready/channel.rs`](src/query/render_ready/channel.rs) 独立输出
`canInviteMembers` 与 `canManageMembers`。邀请只面向 `P` / `G` 类型：优先读取
`can_invite_members` / `canInviteMembers` 显式值；缺省时由已确认的
`CREATOR` / `MANAGER` / `MEMBER` / `BOSS` 角色生成。显式 false 保留，
`O` / `D` / `T` 类型不开放该邀请能力；普通 MEMBER 可邀请不等于可管理成员。
定向回归入口：`cargo test -p helix-im --lib invitation_is_independent_from_member_management`。

## 文字接龙消息补全

[`channel_stream_event.rs`](src/ws/handlers/channel_stream_event.rs) 对未 redacted、
`PostUpsert` 且缺完整 post ID 的 `im:post_chain:*` 事件，通过 `im_get_posts`
读取权威消息；事件 ID 只作去重键，不能替代消息主键。即使专用接龙投影已推进 cursor，
完整消息仍可独立补全。

[`dispatch_core.rs`](src/port_reply/dispatch_core.rs) 校验 `posts/get` 业务状态、请求消息 ID、
频道、`TEXT_CHAIN` 类型、作者、创建时间及可见性后才进入原持久化路径；读取失败不提交
空白消息，恢复交给频道同步。成功 Emit 仍受 matching PersistOk 约束。
定向回归入口：`cargo test -p helix-im --test stream_persist_barrier`、
`cargo test -p helix-im --test http6_sync_reply_unwrap_test chain_sync_replay`。

以上两节仅核对当前实现及回归测试源码；本轮未执行 Rust 测试、真实 Go 或客户端 UI 验收。

## RICH 图片与视频上传进度

[`send/upload_props/request.rs`](src/send/upload_props/request.rs) 对普通文件、RICH 图片与视频
请求 `PercentStep(5)` 字节进度；模板图片仍为 `Disabled`。RICH 逐文件进度写入对应
`files[]` 节点，消息级 `uploadProgress` 按文件 `size` 加权，规则见
[`send/upload_props/state.rs`](src/send/upload_props/state.rs)。

[`port_reply/media_write.rs`](src/port_reply/media_write.rs) 按 temporary id 串行化同消息的
媒体进度、完成与失败写入；在途写入期间保留最新进度，排队的终态回包优先处理。
[`port_reply/progress.rs`](src/port_reply/progress.rs) 仅在匹配的持久化成功回执后更新已提交
进度并触发 timeline 回读。普通发送未携带 window token 时解析当前实际附着窗口；
显式重试 token 仍绑定原窗口，不能把字面值 `latest` 当作当前窗口别名。

定向入口为 `cargo test -p helix-im --test message_v3_mv3_g01c` 与
`cargo test -p helix-driver-host --test media_pipeline_e2e`；本轮仅核对源码和测试入口，
未执行测试，也不据此宣称真实 Tauri UI 上传进度已验收。

## 按观察范围读取在线状态

`im_presence_observe({scope_id,user_ids,focused_user_ids})` 是本地确定性登记命令，
通过公共 `client_api` / `Tick::Command` 进入；同 scope 全量替换，两个数组为空时释放。
每 scope 最多 200 个去重用户，最多 64 scopes；合并后优先 focus，实际观察最多 200 人，
其余明确为 unknown。最多 10 个 focus；v2 订阅覆盖全部实际观察用户，
旧服务端兼容路径仅为 focus 建立 WS 订阅，其余走普通查询。
Host 必须为窗口创建唯一 scope 并在隐藏/销毁时释放。身份与企业由 ImConfig 提供，
不接受 renderer payload 修改。无持久化，不统计群在线人数。

输出为 `{event:"im:presence:update",data:{statuses:{userId:"online"|"offline"|"unknown"}}}`，
覆盖当前已登记用户与仍有效的有界缓存；首次无缓存时产生 unknown，网络确认后才出现
online/offline。短暂释放观察可以保留原期限内的状态，缓存总量最多 200 人。
兼容路径中，新增观察仅补查需要刷新的用户，并避免同用户重复在途；重连重新批查 `/users/status/ids`
（最多 100 人/批）；滚动保留交集在途请求，不重置普通用户的 30 秒刷新周期。
首次周期按认证用户与连接 ID 确定性错峰 0–5 秒；失败按用户退避 30–300 秒，成功清除退避。
普通查询有效期从请求开始计算 45 秒；focus 初始查询有效期 15 秒，较新的 WS 样本优先。
普通与 focus 切换时立即补查，但保留已有有效状态，避免在线绿点先清空再恢复；
晋升 focus 后等待新确认最多保留 15 秒，且不超过原有效期，反复切换不会续活旧样本。
专用查询携带 `includeMetadata:true`，返回 `{statuses,checkedAt}`；普通旧查询仍为 map。
HTTP/WS 按相同服务端 checkedAt 排序。用 HTTP 采样时刻减本地请求发出时刻建立时钟
偏差上界，保守换算 WS 有效期，延迟到达不续活过期状态；首个 HTTP 确认前不晋升 WS，
确认失败通过续订周期有界重试；任何 WS 样本的本地剩余新鲜期最多 15 秒。
每次重连重新校准；同一连接期采用已确认偏差上界的最小值。服务端节点时钟同步及
会话内时钟稳定仍是 Redis 租约与 checkedAt 排序的前提，时钟前跳不能延长本地新鲜期。
兼容 WS 发 `{action:"presence_watch",seq,data:{userIds,generation}}`，每 20 秒续订；
接收 `{action:"presence",data:{generation,statuses,checkedAt}}`，完整快照省略用户即 unknown。
检查失败不延长状态，15 秒未获 focus 确认变 unknown。兼容订阅只有 focus 集合变化或连接恢复
改变代次；普通列表滚动不重发兼容 watch。同代递增 checkedAt 防旧帧覆盖。

v2 使用同一 `presence_watch` action，额外携带 `version:2`、`requestId`、
`generation`、全量 `userIds` 与 `focusedUserIds`，每 20 秒续订。入站必须匹配代次、
未过期请求与连续 `sequence`；首帧必须是 `snapshot`，后续支持 `delta` / `confirm`。
断序会重置代次并请求新快照。有效确认后 40 秒内停止 HTTP 查询并忽略旧协议与在途 HTTP
结果；失联后恢复兼容查询。每用户有效期由 `validUntil` 和请求关联的时钟偏差界确定，
最长 30 秒；缺失或过期授权为 unknown，确认不能无条件续活样本。实现真源为
[`presence.rs`](src/presence.rs)，订阅活性期限与每用户授权期限不可混为一谈。

生命周期：输入非法直接报错且不改变观察；HTTP/Port/业务失败不伪造 offline，保留已确认
状态仅到原期限；请求 15 秒超时后丢弃关联，迟到/重复回复不生效。取消 scope 释放对应用户关联；
最后观察者离开停止查询/续订，但保留缓存到期计时器，后台只清除到期状态。断线清待处理
关联并保留未过期缓存，hello 后新代取快照，不延长旧状态；身份切换或 stop 立即清空缓存
与观察意图。Persist/SQLite 生命周期为 N/A（瞬时状态）；兼容 focus 与 v2 分别按上述
新鲜期/授权截止处理。
定向回放入口：`cargo test -p helix-im --test presence_test`（Cargo 自动发现）；本轮仅核对
源码与测试入口，未执行 Rust 测试。该回放仅覆盖 sans-IO 合同，
真实 Go/WS/Tauri UI、三端业务验收和 5000 人群负载未由这些回放证明。

## 普通未读清除边界与可见回应查询

`im_channels_view` 的 `channels` 每项为 `{id,isRoot:true,throughPostId?}`。
可选 `throughPostId` 必须是 canonical post ID；省略时保留原请求形状，提供时原样
透传至 `/channels/view`。公开入口与 outbound 均校验，null、空串、临时 ID、非字符串
及未知字段均拒绝，不将非法边界降级为无边界请求。见
[`client_api/command.rs`](src/client_api/command.rs) 与
[`outbound/channel_admin/change_dedicated.rs`](src/outbound/channel_admin/change_dedicated.rs)。
命令与 HTTP 成功不乐观清零；频道投影仍由权威成员事实经持久化屏障推进。

`im_reaction_unread({channel_id,req_id,post_ids?})` 将可选 `post_ids` 映射为
`posts/reactionUnread` 的 `postIds`：允许空数组，最多 100 个不重复、非空且无首尾空白的
字符串。省略时不附加过滤字段。ACK 仍使用
`im_reaction_unread_ack({channel_id,post_id,observed_revision,req_id})`，不接收 `post_ids`。
请求合同见 [`outbound/engagement/reaction_unread.rs`](src/outbound/engagement/reaction_unread.rs)。

响应的可选 `targets` 最多 100 项且不超过权威 `count`；post ID 不重复，revision 为正
十进制 i64 字符串。成员投影须匹配频道、认证用户及回应未读数，经原子持久化和版本/effect
读回核对后，才通过 `im:read:result` 原样返回响应 body；失败不发布成功结果。见
[`port_reply/reaction_unread.rs`](src/port_reply/reaction_unread.rs)。

回归入口：`cargo test -p helix-im --test message_v3_g13`；其中包含边界透传、非法过滤、
非法 targets、持久化失败与重复回执用例。本轮仅核对源码与 Cargo 注册，未运行这些测试，
不据此声明 Go、Native/UI 或大群容量已通过。
