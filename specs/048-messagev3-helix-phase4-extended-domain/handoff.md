# MessageV3 Phase 4 三域 Helix 交接

更新时间：2026-08-03

## AGENT_HANDOFF/v1

```text
AGENT_HANDOFF/v1
state: partial
phase: 4
repository: helix
phase_denominator: 16
active_scope: pin, bookmark, announcement
blocked_domains: search, todo
phase2_status: PARTIAL
phase2_canonical: 0/20
merge_commit: 61645f14
next-owner: messagev3-artifact-reviewer -> messagev3-phase4-supervisor（真实跨仓证据与 Phase2 baseline blocker）
stop: 不得推进 Search/Todo；不得把 deterministic/in-process 证据升级为 real/dual-user/Native/canonical PASS；不得重建 Runtime、Port、单总线、Command Result、PostProjection 或 readBits
```

## Phase 2 基础合并

- 来源分支：`origin/codex/messagev3-phase2-review-20260731`
- 来源 HEAD：`9054f98fb0bd5140ce377d8c4d0f21aecbba4913`
- Phase 4 合并前 HEAD：`ddd4e5e6aae1`
- 实际 merge commit：`61645f14`
- Helix 无文本冲突。
- 实际合入范围：`helix-im` 的 `channel_sync`、read/timeline/sync query、absolute projection、
  event/WS handler、hydration 和受影响 `helix-im` tests；未修改 Phase 4 spec/plan/tasks/manifest/
  contract/TARGET_REFERENCE，未触碰 Search/Todo。

## 复用矩阵

| 已有基础 | Phase 4 复用 | 未覆盖 |
|---|---|---|
| Phase 1 Runtime、单总线、Command Result | Pin/Bookmark/Announcement 继续使用既有入口 | 三域真实 command/event parity |
| Phase 2 sync/cursor/hydration、timeline window、分页 | Pin 查询和 Announcement 列表读取复用 | 三域真实 HTTP/WS/Storage、Native、双用户 |
| PostProjection、read receipt/readBits | Pin NOTICE 和公告确认复用既有权威 | 不新增第二套 PostProjection 或 read authority |

Phase 2 日志和测试只证明 Phase 2 观察结果，不证明 Phase 4 runtime、dual-user、Native 或
canonical。完整 Phase 4 分母仍为 `16/16`；当前只授权 Pin、Bookmark、Announcement 的 9 个
Gate，Search/Todo 的 7 个 Gate 继续 `BLOCKED`。

## 当前三个领域源码审计

以下是本轮实施前 Helix worktree 的代码事实，不是跨仓 runtime 或 canonical 证据；实施后差异见下节。

| 领域 | existing | reuse | missing / 状态 |
|---|---|---|---|
| Pin | `im_set_message_top`、`im_set_message_untop` 已构造真实 endpoint/body；`im_channel_load_post_pinned` 已存在；WS `post_pin` handler 会把 `{postId,channelId,operation}` 转成持久化 pinned patch 和 `im:post:updated`。 | Phase 1 Runtime、单总线、PostProjection、`im:post:received` NOTICE 与 `postUpdated`。 | 三仓 command/event parity、真实 Go/WS/Storage、Angular 消费和双用户尚未验证；状态 `PARTIAL`。 |
| Bookmark | `im_bookmark_create/delete/load` 已注册并可构造 `/post/bookmark/*` 请求；本轮已收口 actor-free wire、channel scope 与 page 参数。 | 复用结构化 read result、Phase 2 分页/读取基础，不产生公共事件。 | 真实 Go HTTP/Storage、迁移、双用户和 canonical 未验证；状态 `PARTIAL / RUNTIME_UNVERIFIED`。 |
| Announcement | `im_save_announcement` 公开别名映射到 `im_announcement_save`；本轮补齐 list/delete version projection、`announcementChanged` 消费与绝对列表事件。 | Phase 1 G07b Post authority、PostProjection、read receipt/readers 和单总线。 | 真实 HTTP/WS、双用户、Native 和 canonical 未验证；状态 `PARTIAL`。 |

Helix 当前可以提供三域的 outbound/WS 构造基础，但不能据此宣称 Phase 4 Gate PASS。尤其 Bookmark
的旧 `user_id` wire 形状必须作为显式实施缺口保留，不能由测试绿灯掩盖。

## 本轮 Helix 受限实现（2026-08-04）

本轮只修改 active scope 的 Pin、Bookmark、Announcement 代码与回归资产；未触碰 Search/Todo、
Phase 1/2 基础设施或 Phase4 Spec/Plan/Tasks/manifest/reference JSON。

- Pin：`set_message_top/untop` 现在 fail-closed 只接收 `{channel_id,post_id}`；`post_pin` 只接受
  Go authority 的 `add/remove`，未知 operation 不产生持久化或事件副作用。
- Bookmark：create/delete/load 改为 `{channel_id,post_ids}`、`{channel_id,post_id}`、
  `{channel_id,page_number,page_size}`，请求关联 `req_id` 不进入 wire；`user_id/offset` 等内部字段拒绝。
- Announcement：delete 改为 `{channel_id,announcement_id}`；新增 `announcementChanged` 版本去重、
  list HTTP 重拉、`im:announcement:list-updated` 绝对列表事件，以及 delete 结果的同版本列表重拉。
  list/delete 回包缺失正版本、频道不匹配或版本倒退时只回灌 read error，不推进状态。

源码/单元/回放验证已在本 worktree 通过：`cargo check -p helix-im`、
`cargo test -p helix-im announcement -- --nocapture`、
`cargo test -p helix-im --test read_relay_test announcement_delete_replies_and_reloads_same_version_list -- --nocapture`。
这些证据仅覆盖 deterministic/unit/contract/in-process replay；真实 Go HTTP/WS、Native 宿主、Angular
消费、双用户和 canonical Gate 仍未采集，状态保持 `PARTIAL`。

## 下一 owner 与停止条件

下一 owner 按 active Gate 白名单补真实跨仓证据并复审 Pin、Bookmark、Announcement。若实现要求新建第二套基础设施，
或依赖 Search/Todo 的任何命令、事件、过滤和路由，立即停止并报告精确阻塞点。

## 实施后审查结论（2026-08-04，Phase2 修复前历史记录）

- 当前 worktree 已完成 active Pin/Bookmark/Announcement 的受限 Helix coding 与 no-op delete/version 对齐，未提交、未 push。
- `cargo fmt`、`cargo check -p helix-im`、Pin/Bookmark/Announcement focused 单测与 in-process relay 通过；完整 `read_relay_test` 仍有 2 个 Phase2 基线失败，未修改该共享行为。
- 真实 Go HTTP/WS、Angular/Tauri runtime、Native、双用户和 canonical 证据未采集；当时 reviewer 另因 Phase2 `read_relay_test` 两个失败判定 `BLOCKED/PARTIAL`。该 Phase2 原因已由下方修复记录取代。

## 最终 Artifact Review 对账（2026-08-04，Phase2 修复前历史记录）

- G07d reference sender/viewer 数量已对齐；Go Announcement 鉴权链已复用现有成员 authority 与 NoticePermission，不新增权限系统。
- `cargo fmt --all -- --check`、`cargo check -p helix-im`、Pin/Bookmark/Announcement focused 测试均退出码 0。
- 精确命令当时为 `11/13`：p3a `27/27`、P4 WS `9/9`，仅两个既有 Phase2 `read_relay` 基线失败；该数字已由 Phase2 修复后的当前记录取代。
- 当时 Reviewer 维持 `BLOCKED`（`critical=0, high=1`）。真实 Go HTTP/WS/Storage、双用户、Angular/Tauri/Native、canonical 仍未验证；该 HIGH 不是当前 Phase4 active 代码缺陷。

Search/Todo 继续 `BLOCKED`；该历史记录的下一责任方曾为 Phase2 read relay owner 与跨仓真实证据 owner，不得通过降低 Phase4 合同消除阻塞。

## 当前轮验证增补（2026-08-04，Phase2 修复前记录）

- `cargo fmt --all -- --check`、`cargo check -p helix-im` 以及 Pin/Bookmark/Announcement focused 单测与 in-process relay 均通过。
- 聚合命令当时为 11/13：前两组分别 27/27、9/9；`read_relay_test` 的 2 个失败是当时的 Phase2 基线失败。该结果已被 `9813df07`/`5f3a3888` 的 13/13 结果取代。
- 未取得真实 Go HTTP/WS、Storage、Angular/Tauri/Native、双 persona 或 canonical 证据；Helix 状态保持 `PARTIAL/BLOCKED`。未取得 `IMPLEMENTATION_ADMITTED`，未调用 `speckit-implement` 或 `speckit-converge`。

## 本轮幂等删除与 version=0 收敛（2026-08-04）

- Go Phase4 未命中/重复删除回 `data={channelId,deletedAnnouncementId,version:0,noop:true}`；Angular
  normalizer 同时接受 `noOp`、`no_op`、`noop`，并把 `version===0`（无论显式标记是否为 false）归一成
  `noOp:true`。Helix
  `announcement_list_projection` 允许 Go 初始频道的权威 `version=0`；`announcement_delete_projection`
  接受 version=0 或显式布尔 no-op，并输出 canonical `noOp:true`，仍拒绝非布尔标记。
- `emit_announcement_delete_reply` 对 no-op 只回灌 `im:read:result`，不启动 Announcement list
  reload；正版本只做 `max`，version=0 不降低既有频道版本，保留已有 pending list continuation，
  因此不会产生列表回滚。
- 新增 read_ext 投影单测（version=0、snake no-op、错误类型）与 `read_relay_test` no-op 回放：
  `version=0` 成功回灌 canonical no-op 且无第二条 HTTP。Phase2 read relay 的既有 2 个 baseline
  failure 未修改，仍是独立 blocker。

本轮可复核命令均退出码 0：`cargo fmt --all -- --check`、`cargo check -p helix-im`、
`cargo test -p helix-im --lib announcement_list_projection_requires_version_and_canonical_fields -- --nocapture`、
`cargo test -p helix-im --lib announcement_delete_projection_accepts_version_zero_and_explicit_noop -- --nocapture`、
`cargo test -p helix-im --test read_relay_test announcement_delete_noop_version_zero_replies_without_list_reload -- --nocapture`、
`cargo test -p helix-im --test read_relay_test announcement_delete_replies_and_reloads_same_version_list -- --nocapture`、
`git diff --check`。仅有既有 dead-code warnings；未以完整 `read_relay_test` 的 Phase2 基线结果冒充本轮通过。

证据边界：本轮只验证 deterministic/unit 与 in-process relay；Go/Angular 共享 worktree 源码对账为
contract evidence，未采集真实 Go HTTP/WS、Native、双用户或 canonical Gate，整体仍为 `PARTIAL`。

## Phase 2 read relay 阻塞的因果审计（2026-08-04，Phase2 修复前历史记录）

- 在 Phase 2 review source `c4e3dfc85ef09429a8a4d2ac791bb72f47e0f062` 上独立运行两个失败用例，结果仍分别为失败；因此不是 Phase 4 dirty worktree 引入的测试偶发。
- merge-base、Phase 2 source、Phase 4 HEAD 与当前工作树中两个失败函数的 SHA-256 均为
  `e93f4f1f80d03bf60eefe8f4321f0a2e055c6b8796f7b500c1c42c20c4fe64b7`；Phase 4 diff 未修改失败函数体。
- 根因位于 Phase 2 的 `crates/helix-im/src/module/trait_impl.rs:180-203`：缺少 `req_id` 时提前
  `return Ok(())`，导致已构造的 HTTP effect 未执行；现有测试/投影文档要求“HTTP 可执行、corr
  不注册、不得产生无关联投影”。这属于 Phase 2 G13b 共享合同漂移，Phase 4 不得修复或降级。
- 当前无不修改 Phase 2 代码即可关闭该 HIGH 的准入路径。下一 owner 是 Phase 2 read-relay/hydration
  owner；需要恢复 HTTP fire/no-corr 语义使测试达到 13/13，或由人工批准更新测试/文档合同后重新 Review。
- 当时已刷新 `origin/codex/messagev3-phase2-review-20260731` 至 `4f1a73dc`，该远端仍保留同一缺 `req_id` 分支；该状态已由后续 `9813df07` 修复提交取代。

## Phase2 G13b 修复后的当前审计（2026-08-04）

- `9813df0797875616572808d8ff0494527209050e` 已由 Phase2 owner 提供，并以 `5f3a3888` 合入本 Phase4 Helix worktree。生产修改仅涉及 `crates/helix-im/src/module/trait_impl.rs`，未触碰 Phase1、Phase4 active、Search 或 Todo。
- 当前 aggregate focused 结果为：`p3a_posts_read_test` `27/27`、`p4_ws_events_test` `9/9`、`read_relay_test` `13/13`；`cargo fmt --all -- --check` 与 `cargo check -p helix-im` 均通过。旧 Phase2 HIGH 已关闭。
- 本节是当前事实源；上方标记为“Phase2 修复前历史记录”的 `11/13`、`high=1` 和“无可合入修复”不得继续作为当前准入阻塞。
- 真实 Go HTTP/WS/Storage、Native/DOM、双 persona、canonical 仍未采集，Helix 证据等级保持 `PARTIAL/UNVERIFIED`；不得以本地测试代替这些证据。Search/Todo 继续 `BLOCKED`。

## T4-030 Implement 与 Convergence 交接（2026-08-04）

```text
IMPLEMENT_HANDOFF/v1
state: partial
phase: 4
repository: helix
domains: pin,bookmark,announcement
gates: MV3-G05f,MV3-G05g,MV3-G05h,MV3-G05i,MV3-G05j,MV3-G05k,MV3-G07a,MV3-G07c,MV3-G07d
```

- 本轮 Helix active 代码已由既有 worktree 实现并完成审计；未新增第二套 Runtime、Port、PostProjection、readBits 或事件总线。Phase2 `9813df07`/`5f3a3888` read relay 13/13 保持通过。
- focused `p3a_posts_read_test` 27/27、`p4_ws_events_test` 9/9、`read_relay_test` 13/13、`cargo fmt --all -- --check` 与 `cargo check -p helix-im` 均通过。
- 不将本地 relay/unit/contract 结果升级为真实 Go HTTP/WS/Storage、Native/DOM、双 persona 或 canonical 证据；Search/Todo 继续 `BLOCKED`。

```text
CONVERGENCE_HANDOFF/v1
state: partial
real_evidence: blocked
new_tasks: 0
```

- Convergence 未发现现有 T4-001..T4-030 之外的确定性 Helix 实现缺口，未修改 tasks/Spec/Plan，未追加任务。
- 真实跨仓 round-trip、受控 PostgreSQL、双用户/多设备、Native/DOM 和 canonical 证据尚未采集；下一 owner 为真实证据环境 owner 与 Phase4 Supervisor。Search/Todo、Phase1、Phase2 共享范围保持只读。

## 未来 Roadmap：Search 与 Todo

Helix 本轮交付范围仅为 Pin、Bookmark、Announcement；Search 与 Todo 继续作为后续独立领域，
当前保持 `BLOCKED`，不因本轮 focused 测试或事件接线而提前解锁。

| 领域 | Gate | 后续 Helix 工作 | 当前禁止事项 |
|---|---|---|---|
| Search | `MV3-G14a..c` | 在 Go 正式搜索端点确定后，新增最小 Gateway/Command 编排、统一错误和三段结果协议，并与 Angular 查询投影对账 | 不得假设当前 Go Router 已有正式搜索端点；不得把现有普通查询当全文搜索 PASS |
| Todo | `MV3-G15a..d` | 复用 `im_query_todo_list`、`im:todo:updated` 和既有 Tauri ingress，补 production Gateway 调用、`helixImEvent.adapter` 映射和 persona 过滤 | 不得重建 Todo Core、Runtime、Port 或总线；不得把现有 Core/Host 命中写成端到端完成 |

进入 Roadmap 时必须先完成独立调查、Gate 合同和 Open Decision 收口，再生成 reference，之后按
Spec → Plan → Tasks → Review → Implement → Converge 推进。证据必须分层记录，不能用 Pin/Bookmark/
Announcement 的 relay、Mock、unit 或 TARGET_REFERENCE 代替 Search/Todo 的真实 HTTP/WS/Storage、
双用户、Native/DOM 或 canonical 证据。

若 Search/Todo 任务要求改变 Phase 1/2 的 read relay、PostProjection、readBits、Command Result
或本轮三域事件协议，立即停止并回报跨 Phase 依赖，不得在该领域 handoff 内旁路修改。
