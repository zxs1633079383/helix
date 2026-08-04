# MessageV3 Phase 3 Channel Domain Helix 交接

**协议：** `AGENT_HANDOFF/v1`
**状态：** `PARTIAL`
**更新时间：** 2026-08-03

## 1. Phase 2 基础继承

本 worktree 已将 `origin/codex/messagev3-phase2-review-20260731` 合入 Phase 3 分支。继承的
Read/Timeline/Sync 基础包括：

- `channel_event_cursor`、事件 gap/TooLong、hydration 与 generation 隔离；
- read、timeline、replies、分页和 sync dispatch/render-ready；
- 既有单一 Helix Runtime、Port、Correlation/dispatch、Native EventSink 对接边界；
- Phase 2 的 `ready → page → apply → complete` 仅为 runtime observation，不是 Phase 3 或
  canonical PASS。

Phase 2 状态仍为 `PARTIAL`，canonical `0/20`；dual-user、Native、canonical 未完成。

## 2. 来源与合入

- Phase 2 source branch：`origin/codex/messagev3-phase2-review-20260731`。
- Phase 2 Helix checkpoint：`9054f98fb0bd5140ce377d8c4d0f21aecbba4913`。
- Phase 3 merge 前 HEAD：`4d3f7e67cc33ab2f2a70ec3824b3a9a32b78e2cc`。
- Phase 2 合入父提交：`9054f98fb0bd5140ce377d8c4d0f21aecbba4913`。
- 实际 merge commit：`9ee111bd240ca0c8fc3a4d460dd9fe3f8f184f4e`。
- merge 后 worktree branch：`codex/messagev3-phase3-review-20260731`。

## 3. 实际合入范围

合入 `crates/helix-im` 的 `channel_sync.rs`、`event/channel_sync.rs`、read/timeline/sync
query/dispatch、`query/render_ready`、hydration、state、WS handlers，以及 Phase 2 G06/G11/G12/G13
focused tests；完整文件集合以 merge commit `9ee111bd` 的 `git show --stat` 为准。

## 4. 冲突解决依据

本次冲突位于 capabilities 断言、correlation 类型、query scope 和 port-reply dispatch：

- 保留 Phase 3 channel/member 能力断言，不恢复已禁止的旧事件；
- 同时保留 Phase 2 `SubtopicsQuery`/scope 与 Phase 3 既有成员快照类型；
- 生产 `DialogListQuery` 使用 Phase 2 typed read result；`DialogListMemberSnapshot` 仅保留
  兼容分支，不建立第二套 read pipeline；
- 保持单一 Runtime、Port、Correlation/dispatch、projection 和同步状态机。

## 5. Phase 3 当前边界

Phase 3 Gate 分母仍由 Client 的
`scripts/message-v3/reference/phases/phase3.manifest.json` 唯一决定，为 15 Gate；本次不修改
manifest、contract 或 TARGET_REFERENCE。当前可复用 Phase 1 Foundation 与 Phase 2 read/sync
内核，但真实 Go/HTTP/WS/Storage/Native、双 persona、多设备和 canonical 证据仍未完成。
TARGET_REFERENCE、Mock、Jest、build 或 HTTP 200 不得升级为这些证据层 PASS。

## 6. 验证与下一 owner

- `cargo test -p helix-im` 的 Phase 2/Phase 3 focused targets：PASS。
- `git diff --check`：PASS。
- Phase 2 canonical：`0/20`；Phase 3 仍 `PARTIAL`。

下一 owner 为 Phase 3 编排链；本交接完成后不自动 implement/review/converge。缺 provider、认证、
receiver、真实 DB、Native、独立 persona、第二设备或 runtime provenance 时，保持
`BLOCKED/NOT_RUN`。

`PHASE3_HELIX_HANDOFF_PHASE2_FOUNDATION_MERGED_PARTIAL`

## 7. 独立实施 worktree T012-T019（2026-08-04）

- 当前 worktree：`/System/Volumes/Data/workspace/worktrees/messagev3-phase3-impl-20260804/helix`，
  branch=`codex/phase3-channel-member-20260804`，production commit=`47ec9ee0`。
- `member_admin` 只接受 `NORMAL|STRONG|IGNORE` notify 与 `ADMIN|MEMBER` role；不新增 command、event
  或 storage 表。
- `change_dedicated`/`change` 沿既有 command/correlation/PortReply 链加入 orient 0-30 字符、
  `expected_version` CAS、权限部分补丁和 `BOSS/type=O` 边界。
- `trait_impl` 对显式 `req_id` 的 increment 保持既有 `OutboundReadReply`/`im:read:result` 纯读路径；
  无 req_id 的历史 hydration 兼容路径不改变，未新建第二 Result registry。
- `member_admin` 的昵称、成员变更和 role wire 复用既有事件映射；presence 仍为 zero-event Result，
  缺失/失败归一由 Host/Client 处理为 `unknown`。

确定性验证：

- `cargo fmt --all -- --check`：PASS。
- `cargo check -p helix-im`：PASS（仅既有 dead-code warning）。
- `cargo test -p helix-im --lib outbound::channel_admin -- --nocapture`：21/21 PASS。
- `cargo test -p helix-im --test p2_posts_outbound_test`：18/18 PASS。
- `cargo test -p helix-im --test outbound_commands`：13/13 PASS。
- `cargo test -p helix-im --test p3b_channel_outbound_test`：27/27 PASS。
- G09a-G09e、G10f、G10g manifest targets：各自 Reference contract PASS。

这些结果只证明 Helix deterministic/contract；真实 Go HTTP、WS receiver、Storage、Native、双
persona、多设备和 canonical 仍 `NOT_RUN/PARTIAL`。G09d 的 canonical `channelIncrement` producer
尚未存在，不得为了填补它新增事件或第二套总线。

`PHASE3_HELIX_T012_T019_DETERMINISTIC_PARTIAL`
