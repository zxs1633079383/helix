# Data Model: GS01-GS04 同步恢复

## 1. 术语与层次

| 层次 | 对象 | 真源 | 说明 |
|---|---|---|---|
| 远端事实 | `RemoteAuthorityHead`、`RemoteChannelEvent`、`RemoteFact` | Go/PG | 判断该身份/频道的权威水位与事实。 |
| IM 领域 | `SyncRecoverySession`、`SyncBatchFacts`、`CommittedSyncBatch` | `helix-im` 内存状态 | 不做 I/O；所有转移由 Tick/PortReply 驱动。 |
| 本地持久化 | 事实表、ledger、coverage、cursor | SQLite | 只在原子 commit 成功后声明 committed。 |
| 呈现 | typed Row/VM、`SurfaceChangeSet`、`InteractionFrameV2` | `helix-im` composer | 从 committed facts 读取，不能由 raw event/renderer 改写。 |
| 平台瞬态 | `FrameCache`、directive receipt、ack | PC/mobile shell | 可丢弃但不能派生业务事实。 |

## 2. 领域对象

### SyncRecoverySession

一次身份建立连接后的恢复上下文。

| Field | Type | Rule |
|---|---|---|
| `session_epoch` | opaque monotonic value | 登录/身份切换时新建；旧 action/cache scope 必须失效。 |
| `actor_id` / `tenant_id` | identity | 从受信 host observation 注入，不从 renderer 取。 |
| `connection_id` | opaque id | 关联 WS、HTTP、persist 与 Frame hop。 |
| `phase` | `Idle | Comparing | Syncing | Recovering | Ready | Failed | Blocked` | `Ready` 只可在 authority compare 和所有对应 commit 后进入。 |
| `scopes` | `Map<ChannelId, RecoveryScope>` | 仅保存当前身份可见的频道。 |
| `correlation_id` | opaque id | 贯穿 evidence pack。 |

### RecoveryScope

| Field | Type | Rule |
|---|---|---|
| `channel_id` | stable id | 不能跨 identity 使用。 |
| `local_committed_seq` | `Seq` | 仅取成功 `PersistAtomic` 后 cursor。 |
| `remote_authoritative_seq` | `Seq` | 由 authority head / sync response 取得。 |
| `from_seq` / `to_seq` | `Seq` | 覆盖范围满足 `from_seq = previous + 1`，`to_seq >= from_seq`。 |
| `hydration` | `Known | Pending | Committed | Failed` | unknown channel 在 `Committed` 前无 enabled composer。 |
| `recovery_reason` | tagged union | `Reconnect | Relogin | ServerNotify | Gap | UnknownChannel | Resync`。 |

### SyncBatchFacts

解析后的候选事实批次，尚未对 UI 可见。

| Field | Type | Rule |
|---|---|---|
| `channels` | `Vec<ChannelFact>` | 每个事实归属一个 scope。 |
| `members` | `Vec<ChannelMemberFact>` | 复合身份 `(channel_id,user_id)`。 |
| `messages` | `Vec<MessageFact>` | canonical id / temporary identity 的现有规则保持。 |
| `viewer_states` | `Vec<ChannelViewerStateFact>` | 未读/mention/read 水位只取权威事实。 |
| `events` | `Vec<AppliedEventEvidence>` | 已过连续性、identity、hash 校验。 |
| `coverage` | `SyncCoverageDraft` | 关联本批 `from_seq/to_seq`、计数、hash、corr。 |

**Validation**:

- 不同 channel、actor、tenant 的事实不能混入同一 atomic batch。
- event sequence 不得重复；非 snapshot 批次必须连续，否则走 resync/authority reload。
- `to_seq` 不能小于本批最大 event seq；cursor 不能超过 coverage。
- 解析成功不是 committed；持久失败时整批丢弃并保留前一个 committed state。

### SurfaceChangeSet

`PersistOk` 后由事实变化派生的最小呈现影响集。

| Field | Meaning |
|---|---|
| `runtime` | `connection`、`initial_sync`、`recovery` 全局状态。 |
| `user_surfaces` | dialog list、unread summary。 |
| `channel_bundles` | channel summary + composer 必须同 bundle。 |
| `timeline_windows` | 仅 attach 并命中窗口的 timeline。 |
| `thread_details` | 仅 attach 且命中 reply 的 thread。 |
| `directives` | cache commit 后才可执行的语义指令。 |

**Rule**: `SurfaceChangeSet` 只包含 attach registry 中的 slot；未 attach 不产生虚假 VM。用户 scope 的 dialog/unread/runtime 是不可拆分 bundle。

## 3. 本地 SQLite 持久模型

现存事实表保持其既有 owner 与字段；以下新增对象是 V2 同步恢复的最低可审计面。

### channel_event_ledger

| Column | Type | Constraint / purpose |
|---|---|---|
| `tenant_id` | text | 账户/租户强隔离。 |
| `channel_id` | text | channel scope。 |
| `event_seq` | integer | 单调 event identity。 |
| `event_kind` | text | 可审计的语义分类，非 renderer input。 |
| `message_id` | text nullable | 相关 message 的可选关联。 |
| `event_hash` | text | canonical remote event 摘要，不保存 raw JSON。 |
| `coverage_id` | text | 对应 `channel_sync_coverage`。 |
| `applied_at_ms` | integer | driver 写入的诊断时间，不参与领域判定。 |

**Primary key**: `(tenant_id, channel_id, event_seq)`。

**Indexes**: `(tenant_id, channel_id, coverage_id)` 及 `(tenant_id, channel_id, event_seq)`；不得为远端比对做无界全表扫描。

### channel_sync_coverage

| Column | Type | Constraint / purpose |
|---|---|---|
| `coverage_id` | text primary key | correlation-derived opaque id。 |
| `tenant_id` / `channel_id` | text | 账户与 scope。 |
| `from_seq` / `to_seq` | integer | 此次原子提交解释的闭区间。 |
| `event_count` | integer | 与 ledger 行数和 remote range 对账。 |
| `facts_hash` | text | channel/member/message/viewer/event 的 canonical 摘要。 |
| `correlation_id` | text | 跨 hop evidence key。 |
| `commit_state` | fixed `committed` | 只在同事务落盘，失败不可见。 |
| `committed_at_ms` | integer | 诊断时间。 |

**Unique constraint**: `(tenant_id, channel_id, from_seq, to_seq)`。

**Validation**: 一条 coverage 对应的 ledger 事件数量与 `event_count` 必须一致；同一 channel 的 committed coverage 不可交叠，除非逐 event hash 完全相同的幂等重放，后者不得生成第二条 coverage。

### Existing fact tables and cursor

| Table | V2 rule |
|---|---|
| `channel` | 频道资料、生命周期、排序所需事实先于 UI 被同批写入。 |
| `channel_member` | 以 `(channel_id,user_id)` scope 写入/删除；不跨频道删除。 |
| `message` | canonical/temporary identity 保持既有 upsert/reconcile 规则。 |
| `channel_viewer_state` 或现有等价字段 | 与 unread/read/mention 事实同批。 |
| `channel_event_cursor` | 最后一个 `StorageOp`；只代表同事务写集全部 committed 后的已应用边界。 |

## 4. InteractionFrame V2 呈现模型

### Nine Surface registry

| Surface | Scope | GS01-GS04 影响 |
|---|---|---|
| `message.left-dialog-list` | `User(userId,teamId)` | 同步的最新 preview、排序、频道状态。 |
| `message.unread-summary` | `User(userId,teamId)` | viewer state/未读总数。 |
| `message.timeline` | `Timeline(channelId,windowToken)` | attach 且窗口命中的新/补齐消息。 |
| `message.thread-detail` | `Thread(channelId,rootMessageId)` | attach 且回复命中的补齐消息。 |
| `message.channel-summary` | `Channel(channelId)` | channel/member/permission hydration。 |
| `message.composer` | `Channel(channelId)` | 独立编辑器 VM 和 send ActionBinding。 |
| `message.member-roster` | `MemberRoster(channelId,cursor)` | attach 的成员变更。 |
| `message.topic-list` | `TopicList(parentChannelId,cursor)` | attach 的 topic channel 变更。 |
| `message.participant-receipts` | `ParticipantReceipts(channelId,messageId,kind)` | attach 的回执变更。 |

### ComposerVm

| Field | Rule |
|---|---|
| `channel_id` | 必须等于 `Channel` scope。 |
| `state` | `hydrating | ready | forbidden | failed`。 |
| `draft_policy` | 语义策略，不含平台控件实现。 |
| `actions` | 只含 opaque ActionBinding；`hydrating` 时无 enabled send action。 |
| `capability_epoch` | 与 binding `surface_epoch` 一致，身份/权限/频道生命周期变化时使旧 binding fail-closed。 |

### FrameCache state

| Field | Rule |
|---|---|
| `slot` | `(surface, canonical_scope_key)` 唯一。 |
| `revision` / `epoch` | 由 Helix 发出；renderer 原样回传。 |
| `vm` | 完整 typed VM；不是 raw event。 |
| `pending_directives` | 仅在所有 staged slot commit 后执行。 |
| `ack_state` | `FrameApplied` 后可 ack；directive 可另 ack。 |

## 5. State Transitions

```text
Idle -> Comparing -> Syncing -> PersistPending -> Composing -> Ready
                  -> Recovering -> PersistPending
PersistPending --PersistErr--> Syncing/Failed (cursor and Frame unchanged)
Comparing --local == remote--> Ready (no new facts)
Comparing --local > remote / invalid gap--> Recovering or Blocked
UnknownChannel -> Hydrating -> PersistPending -> Composing(summary + composer) -> Ready
IdentityChanged -> InvalidateOldEpoch -> Comparing(new actor only)
```

The only state transition that makes recovery facts visible is `PersistPending -> Composing` on a matching successful correlated port reply.

## 6. Relationships And Invariants

```text
SyncRecoverySession 1 --- * RecoveryScope
RecoveryScope        1 --- * SyncBatchFacts
SyncBatchFacts       1 --- * channel_event_ledger rows
SyncBatchFacts       1 --- 1 channel_sync_coverage row
CommittedSyncBatch   1 --- 1 SurfaceChangeSet
SurfaceChangeSet     1 --- 1 InteractionFrameV2
InteractionFrameV2   1 --- * FrameCache slot staging entries
```

- `channel_event_cursor.last_event_seq <= max(channel_event_ledger.event_seq)` for a nonempty committed range, and must equal coverage `to_seq` for the completed range.
- A Frame cannot advertise `runtime.recovery=ready/recovered` for a scope whose committed coverage/cursor fails validation.
- An enabled composer send action requires committed channel/member context and current session epoch.
- A renderer never creates a Row, VM, binding, sequence, coverage or cursor from raw events.
