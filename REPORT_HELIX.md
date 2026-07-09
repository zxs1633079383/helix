# REPORT_HELIX

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
