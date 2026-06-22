# P3b worktree-local SESSION §0（并行 G1 防 root SESSION.md 合并冲突，main merge 时回灌根 §0）

✅ DONE P3b channel 写族 outbound + P0-2 系统消息消费编排核验 @2026-06-22 +0800 | commit 本提交 | 33 用例(24 p3b + 9 既有 channel_write) | feat/im-takeover
  ↳ 落地：3 独立 distributed_slice 注册文件（C4 收口，与 P3a posts_read.rs 零交集）：
     `channel_create.rs`(im_create_specify_owner #8 raw 透传 + displayName 校验)、
     `channel_change.rs`(change info/source/picture/notice/top 5 命令 + put_opt_str/val helper)、
     `channel_member_admin.rs`(member role #29 / notify #30 2 命令)；mod.rs append 3 行。
     P0-2 消费编排已在 main 落地（post.rs reconcile+GuardedBump / posts_update.rs 批帧），本 Phase 不重造，
     `p3b_channel_outbound_test.rs` 补性质核验。
  ↳ 验证：cargo test -p helix-im 全绿（p3b 24 + 全仓零失败）；cargo check 四 crate 过；core 零运行时/C3/C6 grep 空；
     3 新文件覆盖 100% lines+functions；structure-gate PASS（46/183/85 ≤300）；clippy P3b 文件零问题
     （helix-core auth/engine 既有 rustc-1.95 needless_return 非本 Phase）。
  ↳ 预期命中（无 DRIFT-PRED）：① quirk casing 真 Go fixture 比对 ✅（direct_key snake / source·picture·notice 用 id 键 / info·top 用 channelId / notify request channelId）② #15 props 不广播 ✅（im_update_channel_props 未注册）③ 反 RMW B5 ✅（他人 post 单条 GuardedBump unread_count+1 守卫 last_root_post_at，无 Get/无全列 upsert）④ DDD 解耦 ✅（channel 命令只产 Effect::Http 不自拼 post 帧）⑤ makeTopic/create 补字段透传保真 ✅。


🎯 EXPECT P3b channel 写族 outbound + P0-2 系统消息消费编排核验 @2026-06-22 +0800 | 赌注 中 | 可逆 是
  ↳ 可证伪预判：
  ① 新增独立 distributed_slice 注册文件（C4 收口，与 P3a posts_read.rs 零交集）：
     `outbound/channel_create.rs`(im_create_specify_owner 透传 raw body + 校验 displayName)、
     `outbound/channel_change.rs`(change info/source/picture/notice/top 5 命令)、
     `outbound/channel_member_admin.rs`(member role/notify 2 命令)；各 ≤300 行。
  ② quirk casing 真 Go fixture 比对（非自造，C5）：
     · createSpecifyOwner body 含 `direct_key`(**snake_case**) 透传不改写为 directKey（ent.Channel json tag）
     · member/change/notify WS payload key=`id`（非 channelId）—— 但本 Phase 是 **outbound** request body，
       notify request body 用 `channelId`（command.UpdateChannelNotify json tag）；`id` 是 WS **响应** payload key，
       属 P4 WS 事件，**不在** P3b outbound 断言（仅注释标注，避免误测）
     · change/notice request body key=`id`（command.ChangeChannelNotice `json:"id"`=channelId）+ `notice`
  ③ #15 change/props 故意不广播：**不新增** im_update_channel_props WS 处理，仅注释标注勿补 WS（断言 outbound 命令存在但无 WS handler 副作用）
  ④ P0-2 系统消息消费编排已在 main 落地（post.rs handler reconcile + GuardedBump unread/lastPost + posts_update.rs 批帧）
     → 本 Phase **不重造**，补 `p3b_channel_outbound_test.rs` 性质测试核验：
     · 反 RMW（B5）：单次 post 帧 → unread bump 产**单条** `Effect::Persist{GuardedBump}`，不含 SELECT / 不含全列 upsert
     · channel handler 只触发不自拼 post 帧（DDD 聚合根解耦）：channel outbound 命令产 Effect::Http 不产 Persist/post 帧
  ⑤ makeTopic/create 补字段：真 curl §2/§4 验证 raw 透传 body 含 rootId/users[]/picture/forceCreate（前端直供，透传保真）
  ⑥ registry 注册数==命令数性质：新增命令全 is_outbound 命中 + 可 dispatch
  ⑦ cargo test -p helix-im 全绿 + cargo check 四 crate + cargo tree -p helix-core grep 空 + im/src 零 ambient time(C3)
  ⑧ 新 outbound 文件覆盖 100% lines+functions
