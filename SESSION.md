## 0. 当前任务

🎯 EXPECT 修复 G05 Native 多目标转发终局 overlay：只改 cses-client 转发 overlay 的关闭/尺寸契约与 focused harness 诊断，不把 createPosts HTTP 受理当成投递成功；可证伪断言：提交后的 overlay 在 Helix terminal causation Frame 到达前后都不会被 backdrop/Escape 隐式销毁，两个目标仍只在 matching WS + PersistOk Frame 后投影为 pending，Angular focused tests 与 tsc 通过 | 赌注 中 | 可逆 是 | @2026-07-22T12:13:00-0700

🎯 EXPECT 并行实现 G08-G12 的专属 Rust query/schedule/timeline/reply 模块与 focused 测试、必要的 cses-client 专属组件测试；可证伪断言：不修改已声明共享 owner 文件，G08 owner-only hydrate、G09 取消/到点 G01 边界、G10 Observation/Action、G11 VM target/directive 无 payload、G12 query graph/G01 reply 依赖均有回归，批量 focused Rust/Angular 验证通过后才更新 task/handoff，不运行 Native E2E | 赌注 高 | 可逆 是 | @2026-07-22T08:18:00-0700

🎯 EXPECT 调用 Spec Kit implement 完成 G04-G16 的 Helix/Go/cses-client 纵切，并在依赖允许时按不重叠 owner/file set 并行；可证伪断言：① 004-016 逐组执行 before_implement admission，测试先于实现且完成 task 才标 `[X]`；② ActionBinding/Tick/authority/PersistOk/完整 Frame/Angular UI 不回退 raw reducer；③ 最后只由 017 构建 final Native candidate，双账号连续 UC04-16 required fail=0/skip=0；④ 每个 UC 产出 handoff.md，记录真实完成、证据、耗时与未决项；⑤ 同一失败签名最多自动修复两次，第三次 NeedsHuman | 赌注 高 | 可逆 是 | @2026-07-22T08:05:00-0700
✅ DONE G04-G16 已在单 Native 拓扑 `mrc-g04-g16-green-1784776617` 连续跑完 19/19 Gate，required fail=0/skip=0；004-017 tasks 全部完成、逐 UC handoff/evidence 已回填，R1-R4 全 PASS 且 loop CLOSED，逐组 analyze CRITICAL=0；前端 tsc/Jest、Go focused packages、Helix core/im 全量与 Web wasm build 通过 | @2026-07-22T21:05:00-0700
⚡ DRIFT-PRED Helix 全量回归额外暴露恢复 `PersistAtomic` ack 后重复 cursor 写与三个旧测试合同漂移，Cargo 调用超过原预算 → 候选教训：最终聚合前先静态检索 `ChannelPersist` 是否仍产二次 cursor Effect，并一次性迁移所有恢复 fixture 到 tenant+actor authority | @2026-07-22T21:05:00-0700
✅ DONE 最终 Helix pin `a2f835fbb4bee7dda62961c55c4fa9a931302d98` 已由单 Native 拓扑 `mrc-g04-g16-final-a2f835f-1784780000` 连续验证 19/19 Gate，required fail=0/skip=0，耗时 186808ms，合并摘要 `693a10120369536d0401ec031d3e1b18bdfdf18ecdc77e73d16c5cec65b8bfba`；G08 首跑的频道重绘吞首击已修复并经 targeted + full rerun 双重验证 | @2026-07-22T21:18:00-0700
⚡ DRIFT-PRED final pin 首轮唯一红灯不是 G08 authority/Persist/Frame，而是 observer header 被增量重绘后菜单首击丢失 → 候选教训：Native 强交互断言必须在目标 row ready 后重新锚定 active channel，并把 DOM 被替换与业务状态不符拆成不同失败签名 | @2026-07-22T21:18:00-0700

🎯 EXPECT 补齐 `004-017` 的 discovery/data-model/contracts/review-ledger/verification 共同治理文件与逐组 Constitution Check，并修正 tests-first、跨仓 worktree、CURRENT/TARGET/GAP、两次自动修复上限及各 Gate 特有语义；可证伪断言：① 14 组治理文件完整且无模板占位；② 004-016 测试任务先于生产实现，017 复现红灯先于 owner repair；③ 14 组 R1-R3 均由 loop_engine 生成当前指纹 PASS；④ 最终逐组 speckit-analyze 无 CRITICAL，若仍有问题则按 Gate 列出而不伪称通过 | 赌注 高 | 可逆 是 | @2026-07-22T07:20:00-0700

🎯 EXPECT 将 017 从只读验收编排升级为有代码修改权限的收敛修复循环；可证伪断言：① Spec 明确不得在 Red/Partial 报告后停止，必须按日志与五面证据定位并修改 Helix/Go/cses-client；② Plan 允许每个修复批次一次增量 Native rebuild，禁止按 UC 重编，并要求最终 candidate 全批重跑；③ Tasks 拆出 Helix、Go、Angular/UI 三类修复、影响面回归、最终全批与 Index-Dual 双尺寸复验；④ 完成条件是 G04-G16 required=0 fail/skip，而非“已发现问题” | 赌注 高 | 可逆 是 | @2026-07-22T06:08:00-0700

✅ DONE 017 已升级为跨三仓 `run -> classify -> repair -> rebuild -> verify -> rerun` 收敛执行包：Spec 新增 10 条强制修复需求和不得 Red/Partial 停止的退出闸门；Plan 明确 Helix/Go/cses-client owner、日志/PG/SQLite/Frame/DOM 六面诊断、repair-batch 构建纪律与 Index-Dual 视觉修复；Tasks 扩为 T001-T035，35/35 均有独立 Loop Task Contract，包含生产代码修复、可证伪回归、增量重建、影响面重跑和 final candidate 全批 Full Green；本轮只更新规格，未运行 cargo/Native/E2E | @2026-07-22T06:18:00-0700

🎯 EXPECT 在 `specs/` 按 Gate G04-G16 拆出 13 组独立 `spec.md/plan.md/tasks.md`，另设只负责任务汇总验收的 UC04-16 E2E 包；每组明确 ActionBinding、Tick、权威事实、PersistAtomic、Surface/完整 VM row、InteractionFrame、Angular 组件与 PD UI 效果；单 Gate 只做 focused 合同/fixture 验证，Rust Native、双账号、远端 PG、本地 SQLite 与 DOM/视觉验收只在全部 Gate 开发完成后统一执行一次；不改生产代码与现有 001-003 Spec | 赌注 高 | 可逆 是 | @2026-07-22T05:35:00-0700

✅ DONE 已新增 G04-G16 共 13 组独立 Spec/Plan/Tasks（每组 10 个细粒度 task）与 1 组只负责汇总验收的 017 E2E 包，共 42 个分组文档，另有 1 个执行索引；每组均覆盖 ActionBinding、Tick/Effect、权威事实、PersistAtomic、Surface/完整 VM row、InteractionFrame、Angular 组件与 PD UI 状态；017 固定一次 Native build 后连续跑 UC04-16，并分列双账号、Go HTTP/WS、远端 PG、本地 SQLite、Frame、DOM 与双尺寸视觉证据；未改生产代码、未运行 cargo/Native/E2E | @2026-07-22T05:52:00-0700

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
