## 0. 当前任务

🎯 EXPECT 将 `rich-v2.json` 收编为 Helix formal pin 内的 InteractionFrame V2 fixture。可证伪断言：① fixture 含完整 runtime、surfaceEpoch、causationId 和 directives envelope；② Rust `InteractionFrameV2::decode` 可直接解码并保留 reply thread、reaction 与 quick actions；③ manifest 显式登记该 fixture；④ 不改生产投影逻辑、driver 或 wire schema | 赌注 中 | 可逆 是 | @2026-07-22T03:41:00-0700

✅ DONE `rich-v2.json` 已进入 Helix fixture manifest，并使用完整 V2 envelope 固定 reply thread、reaction、quick actions 与 topic link；Rust InteractionFrame 合同 17/17、fixture manifest 4/4 通过，未改生产投影逻辑或 wire schema | @2026-07-22T03:46:00-0700

🎯 EXPECT `message.timeline` 的 Helix `MessageVm` 明确投影最多 3 条回复预览、权威回复总数与逐 emoji 计数；Angular message-v4 仅据此渲染话题卡、回复摘要、气泡外表情及 `message.thread-detail` 抽屉，并通过既有 `im_send_message(replied)` 提交抽屉回复；不改左侧会话列表、导航、存储 schema 或原始事件直通 | 赌注 高 | 可逆 是 | @2026-07-22T04:10:00-0700

✅ DONE Helix `MessageVm` 已新增有界 `replyThread` 与逐 emoji `reactions`，回复预览稳定排序并截到 3 条、总数不被预览长度覆盖；聚焦单测 1/1、InteractionFrame 合同 16/16 通过，Angular 组件测试 2/2 与 fast build 通过；真实双用户 WS/持久化回放尚未执行，不晋级 real-chain-green | @2026-07-22T04:58:00-0700

🎯 EXPECT 左侧会话列表 Frame 保留 channel `pictureType=USER` 的 viewer-aware `userIds`（群聊最多 4 个）与 `pictureType=PICTURE` 的图片资源，并继续以最后消息时间作为 `lastActivityAt`；只改 helix-im 头像投影合同/测试，不改排序、未读、网络或存储 | 赌注 中 | 可逆 是 | @2026-07-22T03:10:00-0700

✅ DONE `5a5ff1c` 已让 Dialog Frame 透传 Helix 裁决后的 `userIds`，USER/PICTURE 分支与单聊对端/群聊四宫格均有回归；helix-im 聚焦测试 10/10、InteractionFrame 合同 16/16、cses-client 新 pin Rust lib check 通过 | @2026-07-22T03:38:00-0700

🎯 EXPECT 只新增一张当前离线同步全链路 Mermaid 图；图中明确分开 cses-client 当前 pin 的 v1 实运行路径与 Helix main 的 V2 候选路径，不把候选实现写成已接入 | 赌注 中 | 可逆 是 | @2026-07-22T00:31:08-0700

🎯 EXPECT 只改 helix-im 的发送投影/预览分类与对应测试：P1 PersistOk 后 `message.timeline` Sending Frame 先于 posts/create，WS/HTTP 终态持久化后刷新同一 slot；`messageClass=normal|mention|urgent` 且 `lastPost.type=TEXT` 始终投影 text preview；不改 cursor、driver 或平台 reducer | 赌注 高 | 可逆 是 | @2026-07-22T01:48:00-0700

✅ DONE `410e9ea` 已补齐 P1 Sending、HTTP/WS/失败终态的 attached `message.timeline` 回读，并让 preview kind 优先读取 `lastPost.type`；Helix 单测 301/301、发送集成 66/66、PC repin 后 Rust lib check 均通过；全量测试仍被既有 `c2_phantom_readbits_test` 缺 tenant/actor identity 阻断 | @2026-07-22T01:52:20-0700

🎯 EXPECT 已确认本地写后的 `message.timeline` refresh 只允许一个有界 message Scan 和 Frame，不得产生 `im_get_latest_post` HTTP；普通文本发送仍保持 P1 Frame 先于唯一 posts/create，WS/HTTP/失败终态不重复出网查最近消息；只改 helix-im query correlation/测试，不改公开协议与 driver | 赌注 高 | 可逆 是 | @2026-07-22T02:18:00-0700

✅ DONE `9dfc728` 已将 durable-fact timeline refresh 收紧为 local-only Scan + Frame，P1/HTTP/WS/失败终态不再 fallback `getLatestPost`；显式 renderer query 保留远端补窗；红灯复现后 Helix 302/302、聚焦集成 39/39、PC 四 crate repin、契约门禁与 Rust lib check 均通过 | @2026-07-22T02:22:45-0700

🎯 EXPECT `message.composer@Channel` attach 必须以 O(1)、零 HTTP/Storage 的 Helix typed snapshot 结束 `awaiting-snapshot`；在 V2 message.create authority 尚未接线前只签发明确 disabled 的发送 binding，不伪造 live capability；补齐 projector/revision/session reset 与 surface resync 回归测试，随后回钉 PC | 赌注 高 | 可逆 是 | @2026-07-22T02:42:00-0700

✅ DONE `67d1ba1` 已为 `message.composer@Channel` 增加 O(1)、零 I/O typed snapshot 与 identity-scoped revision projector，首帧不再落入 surface/scope mismatch；未接线的 V2 send binding 保持 disabled；红灯复现后 Helix lib 304/304、合同/查询 29/29、PC repin/门禁/check PASS；全量仍仅被既有 `c2_phantom_readbits` tenant/actor fixture 阻断 | @2026-07-22T02:49:09-0700
