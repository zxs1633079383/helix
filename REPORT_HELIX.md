# REPORT_HELIX

> ⚠️ 历史报告：其中 `too_long drop-and-reload`、`DeleteWhere` 与
> `JsonListAppendSpec` 结论已在 2026-07-14 architecture drift review 中被判定为
> 违反现网覆盖式同步真源与 HX-C005，并已回正。当前真源以代码、
> `docs/specs/2026-07-09-dialog-projection-and-sync-reset-spec.md` 的回正说明及最新 review ledger 为准。

## 启动自检

- pwd: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709`
- branch: `codex/dialog-projection-sync-spec-20260709`
- baseline: `8e17a0d700ddc8998d57b06e392a3616fbf29d0c`
- 自检结果: 通过

## 完成内容

- 在线 `post + event_seq` 放行后新增 Dialog 指令化投影：`im:channel:update-by-post` 保留 legacy 三键，同时输出 `dialogPatch`、`messageClass`、`senderUserId`、`lastPost`、`lastMessage`、unread/mention/urgent delta、@/加急 userId 列表。
- `PostFields` 扩展并在 parser 一次提取 `simpleMessage`、`userSnapshot`、`mentions`、`expediteMap`；业务判断留在 helix-im 投影层，没有推给 cses-client/mobile。
- viewers/phantom 可见性在在线 post gate apply/flush 两路复核；不可见 post 只推进 cursor，不落可见 message，不 emit `im:post:received` / Dialog patch。
- 离线 sync type1 可见消息复用同一 `PostChannelUpdate` 口径，产出 message emit + Dialog patch；phantom 仍只推进水位。
- `too_long` 从“只发控制信号”升级为 drop-and-reload：按 channel 删除本地 `message` 派生行、reset channel Dialog 派生字段，并 emit `mode=drop_and_reload/dropLocalMessages=true`。
- core/driver storage 增加 `DeleteWhere` 原语；`GuardedBumpSpec` 增加 `extra_bumps`，让 unread/mention/urgent 在同一 guard 下原子自增。
- 同步更新投影 schema 文档、design README、too_long 矩阵说明与相关冻结测试。

## 主要文件

- `crates/helix-im/src/ws/handlers/post.rs`
- `crates/helix-im/src/channel/gate.rs`
- `crates/helix-im/src/channel/write/post.rs`
- `crates/helix-im/src/acl/{projection_control_effects,storage_effects,sync_effects,to_effect}.rs`
- `crates/helix-im/src/sync/{flow,session}.rs`
- `crates/helix-im/src/ws/parser/core.rs`
- `crates/helix-core/src/{effect.rs,ports/storage.rs,ports/mod.rs}`
- `crates/helix-driver-{host,native,wasm}/src/storage.rs`
- `crates/helix-im/tests/dialog_projection_test.rs`
- `crates/helix-driver-host/tests/storage_conformance.rs`
- `crates/helix-im/docs/projection-schema.md`

## 验证

- `cargo check -p helix-im` PASS
- `cargo check -p helix-driver-host -p helix-driver-native` PASS
- `cargo test -p helix-im --test dialog_projection_test -- --nocapture` PASS
- `cargo test -p helix-im --test projection_schema_test -- --nocapture` PASS
- `cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture` PASS
- `cargo test -p helix-im --test s6_sync_internal_test -- --nocapture` PASS
- `cargo test -p helix-im --test http6_sync_reply_unwrap_test -- --nocapture` PASS
- `cargo test -p helix-im test_gate_buffer_capped_proactive_too_long_e6 -- --nocapture` PASS
- `cargo test -p helix-driver-host --test storage_conformance guarded_bump_extra_counts_share_the_same_guard_s3 -- --nocapture` PASS
- `cargo test -p helix-driver-host --test storage_conformance delete_where_deletes_only_matching_scope_rows -- --nocapture` PASS

## Concerns

- `dialogPatch.unreadCount/mentionCount/urgentCount` 当前是本事件 patch/delta 语义，不是写后读取 channel 行得到的累计终值；代码中同时输出 `unreadDelta/mentionDelta/urgentDelta` 明示增量。若前端必须拿累计终值，需要后续新增写后 `Get(channel)`/returning 效果链，不能在当前热路径里隐式 read-modify-render。
- `readLastSeq/msgCount` 当前输出 `null`；Helix 当前 `channel_member` schema 没有 `read_last_seq/msg_count` 类列，不能在不迁移 schema/复合键更新原语的情况下可靠投影。
- 未写 `NEED_<OTHER_PROJECT>_FIX.md`：本轮没有发现必须由 cses-im-server/cses-client 立即补字段或 endpoint 才能完成 Helix 可做部分。

---

## Fix Worker 修复补充（2026-07-09 19:50:49 +0800）

### 启动自检

- pwd: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709`
- branch: `codex/dialog-projection-sync-spec-20260709`
- 自检结果: 通过

### 修复内容

- Critical 1：`im:channel:update-by-post` 改为 `GuardedBump -> Get(channel)` 写后读回链路，`dialogPatch.unreadCount/mentionCount/urgentCount` 只取 Helix storage 累计终值；事件增量保留在 `unreadDelta/mentionDelta/urgentDelta`。
- Critical 1：`mention_list/urgent_post_list` 改为 `JsonListAppendSpec` set-union 语义，并用 set size 写回 `mention_count/urgent_count`，避免覆盖成 `[当前 post_id]`。
- Critical 2：`too_long` 除 drop/reset/emit 外，Helix 主动发起 `im_get_latest_post` 首屏重拉；回包后落 `message`、更新 `channel`，再写后读回并 emit `source=too_long_reload` 的覆盖 DialogPatch。
- Important 3/4：sync type1 和在线 post 统一本地复核 viewers/visible gate；不可见 body 不落可见 Message、不发 UI patch；sender 自己可见，own echo 更新 lastPost 但 unread_delta=0。
- Important 5：从 render-ready DialogPatch 移除 `readLastSeq/msgCount` 的 `null` 占位，并新增 `NEED_HELIX_CHANNEL_MEMBER_FIX.md` 记录 channel_member read/count schema 与写路径缺口。

### 验证

- `cargo check -p helix-im` PASS
- `cargo check -p helix-driver-host` PASS
- `cargo test -p helix-im` PASS
- `cargo test -p helix-driver-host` PASS（保留既有 `network_shared.rs` unused_mut warning）
- `cargo test -p helix-core` PASS
- `cargo test -p helix-driver-host --test storage_conformance -- --nocapture` PASS
- `cargo test -p helix-im --test dialog_projection_test -- --nocapture` PASS
- `cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture` PASS
- `cargo test -p helix-im --test s6_sync_internal_test -- --nocapture` PASS
- `cargo test -p helix-im --test stress_test -- --nocapture` PASS
- `git diff --check` PASS

### 剩余风险 / NEED

- `NEED_HELIX_CHANNEL_MEMBER_FIX.md`：`readLastSeq/msgCount` 仍需后续补齐 Helix `channel_member` schema、send/read/sync 写路径与写后投影后再恢复输出。
- `too_long` 重拉当前使用 `getLatestPost` 首屏覆盖路径；如果后端回包不是 `data.posts` / `data.items` / `data.list` / `data` / `posts` / `items` 数组形态，会安全跳过 emit，需要后续用真实 Go 回包 fixture 继续收紧。
