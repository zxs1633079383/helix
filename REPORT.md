# Message-V4 G-S1~G-16 Helix 投影审计

## 结论

状态：`DONE_WITH_CONCERNS`。

提交：`3cd87d1188b5445891bd3d00af0277a7f8e29af6`（基于既有证据查询提交 `48cbcfd6ba6b17e10181a93bd382e51b6c5c2052`）。

本轮补齐了 message-v4 纯渲染直接需要的消息动作能力、双视角 `isSelf`、服务端消息锚、反应/模板/加急绝对计数，以及 G-16c 群设置回声投影。Helix 单元与关键集成测试通过，但没有在本 worktree 内运行 444/678 Native 双账号活体链路，且 G-03、G-05、G-14 和频道治理权限仍存在跨仓证据或契约缺口，不能据此宣称 27 Gate `canonical-green`。

## 调用链审计

- 在线写回：`ImModule::dispatch_ws_frame -> WS registry -> post/post_update/update_channel/channel_member_update -> StorageOp -> im:__bus__ projection`。
- 离线回放：`handle_sync_reply -> sync_mutation_emits_with_auth -> message_item_data -> im:post:*`，当前用户身份由 `ImConfig.auth_user_id` 注入。
- 本地首屏：`im_query_messages_by_channel -> Scan(message) -> handle_port_reply -> shape_message_rows_for_viewer -> im:messages:query_result`。
- 上拉历史：`im_load_older_context -> posts/postContext -> LoadOlderState -> shape_message_rows_for_viewer -> im:messages:older_loaded`。
- 频道设置：`im_channel_change_info -> Go update_channel WS -> channel partial persist -> im:channel:settings-updated`。
- 复杂度：单消息交互计数对 `quickReply/template/recipients` 为 O(k)；批量消息整形为 O(n+k)，去重 HashSet 均摊 O(1)，总计 O(visible)，无消息全表嵌套扫描。

## 逐 Gate 结论

| Gate | Helix 阶段结论 | 关键命令/投影或缺口 |
|---|---|---|
| G-S1 | ready-unit | hello、channels loaded、cursor 恢复链已存在；需 Native 冷启动重跑 |
| G-S2 | ready-unit | sync + post received + dialog absolute patch 已存在 |
| G-S3 | ready-unit | pong gap 补偿、too-long、恢复投影已有测试 |
| G-S4 | ready-unit | ensure hydration/read-result 已存在；hydration 前 UI 禁用由 Angular 消费状态验证 |
| G-01 | improved | sending/failed/received 现在统一输出 `serverId/isSelf/actionCapabilities` |
| G-02 | ready-unit | `DOCUMENT + props.document` 原样落库与投影；内容 discriminant 仍由 `type` 提供 |
| G-03 | concern | rich/file `UploadFile`、success/failed/retry 状态机与测试存在；但上传中每附件 render-ready projection/Native 上传活体证据未闭环 |
| G-03a | ready-unit | failed/retry 复用 temporaryId，失败 patch 现在明确 `canRetry=true` |
| G-04 | ready-unit | revoke 在线 `post:batch-updated/deleted` 已存在 |
| G-04a | ready-unit | sync revoke replay fat projection 已冻结 |
| G-05 | concern | `im_create_posts`/relay 与逐目标 `post:received` 已存在；缺统一 `targetChannelId -> success/failed` 批结果，无法权威呈现 partial 汇总 |
| G-06 | improved | 新增 `reactionCount` 绝对计数，来自 quickReply.userIds |
| G-07 | improved | 新增 `expediteMap/urgentTargetCount/urgentConfirmedCount` 终态 |
| G-08 | ready-unit | schedule-created + hasSchedulePost=true |
| G-09 | ready-unit | schedule-canceled + hasSchedulePost=false |
| G-10 | improved | 本地查询按 viewer 生成 `isSelf/actionCapabilities` |
| G-10a | improved | older-loaded 同样按 viewer 整形，不再丢动作能力 |
| G-11 | ready-unit | 当前 render-ready event 是 `im:post:located`，不是旧 query_result 猜测 |
| G-12 | ready-unit | `im:channel:replies{reqId,replyIds}` 已存在 |
| G-13 | ready-unit | channel read、absolute dialog count/read echo 已存在；需 Native 双端重跑 |
| G-14 | concern | `readBits` 权威持久化/投影存在；回执人员 popover 尚无独立 render-ready `readerIds/readers`，不能让 Angular自行解释位图 |
| G-14a | improved | 新增 `templateConfirmedCount` 绝对计数，保留 `templateReceived` bool |
| G-15a | ready-unit | create -> channel-created/member projections |
| G-15 | ready-unit | makeTopic -> channel-created/topic fields |
| G-16 | ready-unit | closed 与 left 已分离为不同终态事件 |
| G-16b | ready-unit | member-updated + members/leaves，leaves 是绝对成员 id 列表 |
| G-16c | improved | 新增 `im:channel:settings-updated{channelId,settings,settingVersion}`；仅服务端 WS 回声触发 |

## 新增/变更投影

- 消息 fat 投影新增：`serverId`、`isSelf`、`actionCapabilities`、`reactionCount`、`templateConfirmedCount`、`expediteMap`、`urgentTargetCount`、`urgentConfirmedCount`。
- 乐观发送和失败补丁新增相同动作合同；失败态 `canRetry=true`，sending 态危险动作全部 false。
- viewer 身份贯穿在线 gate、离线 sync、本地首屏和上拉历史。
- 新增频道设置终态 `im:channel:settings-updated`；旧 `im:channel:update` 仍保留兼容。
- projection schema 从 23 工厂/22 channel 更新为 24 工厂/23 channel，并重生成 F5 golden。

## 剩余 Helix/跨仓缺口

1. **G-03**：需要把上传节点 `upload.status/objectKey/url` 形成上传中、失败、成功的 render-ready projection，并用 Native host 真上传验证；当前只有 core Effect/落库测试。
2. **G-05**：需要 Go/Helix 明确多目标 createPosts 的逐目标响应/失败合同，或提供 batch result projection；不能由 Angular 根据收到几条消息自行推断整体成功。
3. **G-14**：需要后端读回执读接口或 Helix render-ready reader projection；`readBits` 不是可让 UI 自行解码的人员列表合同。
4. **治理权限**：本轮 `actionCapabilities` 只覆盖消息级且以本人/服务端锚为依据；关闭群、离群、改设置等频道级 capability 仍需结合 owner/admin/member projection 下沉，Angular 不应自判角色字符串。
5. 本轮没有运行 Go/WS/SQLite/DOM/视觉同一次 run 的 444/678 Native 证据链，所有 `ready-unit` 只代表代码与单测层。

## 验证

- `cargo test -p helix-im --lib`：PASS，200 tests。
- `cargo test -p helix-im --test projection_schema_test --test s1_ws_actions_test --test post_action_inbound_test --test p3c_load_older_context_test --test f5_golden_replay`：PASS；39 + 21 + 7 + 14 + 2 tests，F5 另有 1 ignored fixture writer。
- `git diff --check`：PASS。
- 全量 `cargo test -p helix-im --lib --tests --no-fail-fast` 早期审计发现既有 `p3a_posts_read_test::get_replies_page_defaults_and_revoke_omitted_and_requires_id` 期望 pageNumber=0、实现默认=1 的历史漂移；与本轮变更无关，聚焦回归均已通过。

## 文件

改动均位于 `crates/helix-im/**`；已有证据查询 commit `48cbcfd` 另改 `crates/helix-host-cli/**` 与 `.agents/tools/**`。本报告是 worktree 协调文件，不触碰根 `AGENTS.md/SESSION.md/README.md`。

---

## 第二轮：消费 Go render-ready 合同

状态：`DONE_WITH_CONCERNS`。

提交：`165542e`（`feat(im): 消费消息批结果、回执与频道权限合同`）。

### 已完成

1. G-05：`im_create_posts` 带 `req_id` 时注册 HTTP 回报，新增 `im:posts:batch-result`。逐目标输出 `acceptanceStatus=accepted/failed`；accepted 的 `deliveryStatus` 固定为 `pending`，`deliveryAuthority=ws-post`，不把队列接收冒充最终送达。
2. G-14：`im_post_read_list` 改为双 emit，保留通用 `im:read:result`，新增 `im:post:readers{reqId,receipts}`；只消费 Go 的 `readerIds/readers`，投影中不出现 `readBits`。
3. G-16/G-16c：增量和成员帧记忆当前 viewer 的服务端角色；`update_channel` 使用 Go `capabilitiesByRole` 选择最终 `channelCapabilities`，缺角色或矩阵时全 false。`settingVersion` 优先使用 Go 回声，缺失才回退 WS event seq。
4. 消息动作能力新增 `canUrgentConfirm/canMarkRead/canViewReceipt/canTemplateConfirm`。加急确认按 viewer 的 recipient status、模板确认按 template userIds、回执查看按 sender 身份生成；乐观和失败行全部 fail-closed。

### 调用链

- G-05：`im_create_posts -> Effect::Http -> CorrelationContext::OutboundCreatePosts -> PortReply -> im:posts:batch-result`。
- G-14：`im_post_read_list -> OutboundPostReaders -> im:read:result + im:post:readers`。
- G-16：`increment/channel_member_update -> viewer_channel_roles -> update_channel(capabilitiesByRole) -> im:channel:settings-updated(channelCapabilities)`。
- 消息动作：在线/sync `message_item_data` 与本地/历史 `shape_row` 共用 `message_capabilities::build`。

### 验证

- `cargo check -p helix-im`：PASS。
- `cargo test -p helix-im --lib --test one_line_server_contracts_test --test projection_schema_test --test s1_ws_actions_test`：PASS（200 + 3 + 40 + 21）。
- `cargo test -p helix-im --test f5_golden_replay`：PASS（2，1 ignored writer）。
- `cargo fmt -p helix-im -- --check`：PASS。
- `git diff --check`：PASS。

### 剩余边界

- G-05 的 HTTP accepted 仍需逐目标 WS `post` 才能转 delivered；本轮明确保持 pending，不越权推断。
- G-03 对象存储上传计划仍无服务端 owner，本轮未触碰。
- 未运行 444/678 Native 双账号活体与 Angular DOM，因此本轮仍不是 `canonical-green`。
