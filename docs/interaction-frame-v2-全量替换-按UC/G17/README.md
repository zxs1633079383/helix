# G17 左侧会话列表增量投影

> **命名冲突警告：这里的 G17 只是文档序号，不是 canonical `G-17`。canonical `G-17` 已绑定“频道置顶”（`im_channel_change_top` / `channel.row.v1` / `set_channel_top`），绝不能被本提案复用或覆盖。**
>
> 本文只冻结 V1 现状、V2 目标合同和迁移门槛，不声明实现或验收完成。

## UC 摘要

| 项目 | 内容 |
|---|---|
| 文档序号 | G17 |
| canonical `G-17` | 已用于“频道置顶”，不属于本提案 |
| 本提案机器 Gate | `CONTRACT_DRIFT`：当前无 exact Gate |
| 生命周期状态 | `design-blocked`，等待 Strict Matrix 分配 Gate、render kind 与 required props |
| 用户目标 | 会话发生单行变化时只传输并应用受影响的行，同时保持列表排序、分页和未读摘要一致 |
| 用户角色 | 当前登录用户 |
| 观察者角色 | 同一频道内其他受影响成员；每名成员获得各自 user scope 的个性化增量 |
| Angular 入口 | 无直接用户入口；由已 attach 的 `message.left-dialog-list` 接收投影 |
| ActionBinding 或观察输入 | `HostObservation` / `Inbound`；无新增 ActionBinding |
| Helix Command | 初始或恢复仍为 `im_query_dialog_list`；在线增量没有新增业务 Command |
| 当前 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2`，V1 已物理删除 |
| 目标 Frame 版本 | `InteractionFrameV2`，`schemaVersion=2`，新增 typed dialog row delta |
| 涉及 Surface | `message.left-dialog-list`、`message.unread-summary` |
| Angular 组件 | `message-surface-runtime-view`、`message-ui-surface-dialog-list` |
| Go HTTP | 不新增 G17 专用 HTTP；缺失单频道事实时只允许按 `channelId` hydration |
| Go WS | 复用 `increment_channel`、`update_channel`、`channel_close`、`channel_member_update` 等权威事件 |
| 远端 PG | `channel`、`channel_member`、`post`、`channel_topic`、`channel_event` |
| 本地 SQLite | `channel`、`channel_member`、`message` 与 cursor/projection state |
| 当前复杂度 | 在线状态更新按 key 命中，但 `interaction_view()`、JSON payload 和 Angular replacement 都是 `O(n)` |
| 目标复杂度 | wire `O(k)`；原子 stage `O(k + shifted)`；snapshot/resync 保留 `O(n)` |
| Operation visibility | 在线频道事实为 `multi-party`；每个下发 Frame 的 scope 是 `self-only` user scope |
| 验收状态 | 未绑定 Gate，禁止 promote |

## CONTRACT_DRIFT

当前 Strict Matrix 已有 canonical `G-17`，但它的语义是“频道置顶”，不是 dialog row delta。现有 `G-S1` 只覆盖 `dialog.list.v1` snapshot；最接近在线同步的 `G-S2` 使用未完整登记 required props 的 `message.left-dialog-list.v1`，且与本目录矩阵记录存在漂移。当前没有任何 exact Gate 定义 dialog row delta 的 `renderKind`、required props 和 allowed action。G17 文档中的目标 payload 是待登记合同，不得被当作已发布 wire。

在以下条件全部满足前保持 `design-blocked`：

1. Strict Matrix 新增一个不与 canonical `G-17` 冲突的 exact Gate，并明确这是 observation-driven projection，不伪造 ActionBinding。
2. Helix 与 Angular 同时登记 row delta 类型、版本、负向校验和 snapshot fallback。
3. `message.left-dialog-list` 与 `message.unread-summary` 的同帧原子关系写入 Gate。
4. actor/observer、late-entry、revision gap、分页边界和 refresh replay 证据齐全。

## V1 与 V2 核心对比

| 维度 | 提案切换前事实 | V2 目标 |
|---|---|---|
| envelope | 正式 writer 为 `schemaVersion=1` | `InteractionFrameV2 schemaVersion=2` |
| dialog patch | `DialogListPatch { replaceView: DialogListVm }` | `DialogListDeltaPatch { operations, sectionPages }` |
| 单行更新 | Helix 内部已生成 `UpsertRow/MoveRow`，随后丢弃 | `upsertRow/removeRow/moveRow` 进入正式 wire |
| 排序 | Helix 重建全部 section | Helix 给出 section 与稳定相对位置，Angular 不排序 |
| 分页 | 每帧替换全部 section page | 只更新受影响 section 的 page token/boundary |
| unread | 同帧发送完整 `UnreadSummaryVm` | 继续使用小型 replaceView，但与 dialog delta 原子提交 |
| gap | `baseRevision` 不匹配则 resync | 相同；另加 epoch、anchor、window token 和结果不变量校验 |
| payload | `O(n)` | 常态 `O(k)`，snapshot/resync 仍为 `O(n)` |
| Angular apply | `viewForMutation()` 直接换 VM | clone/stage 受影响 section，校验后一次 swap |

## Operation

| Operation | 可见性 | audience | 输入 | 结果 |
|---|---|---|---|---|
| O1 初始 attach / resync | `self-only` | 当前 viewer | `im_query_dialog_list` 或 resync | 完整 snapshot 建立 revision、epoch 与 window token |
| O2 在线行变化 | `multi-party` | 所有受频道事实影响的成员，每人独立 user scope | Go WS / 本地权威事实 | `upsertRow/removeRow/moveRow`，必要时 paired unread |
| O3 单频道 hydration | `self-only` | 缺失该频道事实的 viewer | targeted `channelId` query | 对已有 snapshot 生成 delta；无 snapshot 时等待 O1 |
| O4 gap 恢复 | `self-only` | 检测到 gap 的 viewer | base/epoch/anchor/window 校验失败 | 整帧拒绝，slot 进入 `resync-required`，重新执行 O1 |

## 完整路径摘要

Go 权威频道事实或本地 hydration
→ Helix `DialogListFrameState`
→ keyed row update
→ 计算受影响 section 与稳定位置
→ paired `DialogListDeltaPatch + UnreadSummaryPatch`
→ `InteractionFrameV2`
→ Angular exact codec
→ copy-on-write stage
→ revision/epoch/window/invariant 校验
→ 原子 swap
→ `message-ui-surface-dialog-list`
→ `FrameApplied`。

## 当前源码证据

- canonical `G-17` 已绑定频道置顶：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:2010`
- `G-S1` 只定义列表 snapshot：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:249`
- `G-S2` 使用尚未完整登记 required props 的 render kind：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/contracts/strict-matrix.v1.json:309`
- V1 patch 正式 wire 仍是 `replaceView`：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:560`
- Helix 内部 row operations 已存在但被丢弃：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:568`
- 全量 `interaction_view()` 遍历所有缓存行：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:692`
- 初始列表查询无 `channel_id` 过滤：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/query/core.rs:228`
- V2 envelope 已存在，但 `SurfaceMutationV2` 仍 flatten 当前 mutation：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/interaction_frame.rs:554`
- Angular 当前明确只接受 `replaceView`：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:421`
