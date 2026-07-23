# G17 Helix 命令与 Frame 合同

## 入口性质

G17 不新增用户 Command。它消费以下事实：

- 初始 attach / resync：`im_query_dialog_list`。
- 在线：`Tick::Inbound` 中的频道、成员、消息、关闭、置顶、已读与话题绝对事实。
- 缺行 hydration：按 `channelId` 的 targeted query/PortReply。
- 本地恢复：`Tick::PortReply` 或 cursor replay。

当前没有 exact Gate、render kind、required props 或 allowed action，因此为 `CONTRACT_DRIFT`。

## V1 当前事实

`DialogListFrameState` 已维护：

- `rows_by_channel: HashMap<String, DialogListRowVm>`
- `ordered_channel_ids: Vec<String>`
- `UpsertRow`
- `MoveRow`

但 `emit_patch_with_causation()` 把 operations 放入 `_legacy_patch` 后不发送，而是调用 `interaction_view()` 生成完整 `DialogListVm`，再输出 `ReplaceView`。

这意味着：

- 正常在线事件没有再次执行整表 SQLite scan。
- `reposition()` 是 `retain + position + insert`，当前为 `O(n)`。
- `interaction_view()`、序列化和 wire payload 为 `O(n)`。
- 初始 `im_query_dialog_list` 仍是无 `channel_id` 过滤的 bounded scan，这是 snapshot 路径，不属于在线 row delta。

## V2 目标 mutation

拟新增独立类型，不能复用 V1 `ReplaceView<T>` 冒充：

```rust
enum DialogListDeltaOperation {
    UpsertRow {
        channel_id: String,
        row: Option<DialogRowVm>,
        set: DialogRowSet,
        clear: Vec<DialogRowClearKey>,
    },
    RemoveRow {
        channel_id: String,
    },
    MoveRow {
        channel_id: String,
        section: DialogSection,
        position: DialogRelativePosition,
    },
}

struct DialogListDeltaPatch {
    kind: DialogListDeltaKind,
    operations: Vec<DialogListDeltaOperation>,
    section_pages: Vec<DialogSectionPagePatch>,
}
```

`TypedSurfaceMutationV2` 或等价 V2-only union 应拥有 `DialogListDeltaPatch`。V1 `TypedSurfaceMutation::DialogListPatch` 继续只允许 `ReplaceView`，避免 schema 1 reader 误收新语义。

## 排序合同

1. 排序权只在 Helix。
2. `MoveRow` 使用 section + `start/before/after` 稳定锚点，不使用跨分页脆弱的绝对 `toIndex`。
3. pinned、normal、closed 是互斥 section；跨 section 变化由一次 upsert + move 表达。
4. bounded window 内需要挤出边界 row 时，Helix 同帧发送对应 remove 与 page metadata。
5. 变化仅发生在不可见窗口且不改变当前窗口边界时，可以不发 dialog mutation。
6. 无法证明相对位置或窗口边界时，退回 snapshot，不发送猜测 delta。

## revision、epoch 与 fallback

| 条件 | 处理 |
|---|---|
| `baseRevision == slot.revision` 且 epoch 相同 | 允许 stage |
| base gap | 整帧拒绝，触发 snapshot |
| epoch mismatch | 丢弃旧 generation，重新 attach |
| anchor 缺失 | 整帧拒绝，触发 snapshot |
| window token mismatch | 整帧拒绝，触发 snapshot |
| duplicate transaction | 幂等忽略 |
| operation 超限或结果不合法 | fail-closed，不部分应用 |

## paired unread 原子提交

频道 row 的 unread/mention/urgent 变化必须与 `message.unread-summary` 同一 `transactionId` 发布。G17 不要求 unread summary 也改成 delta，因为它只有固定小计数字段；可以继续使用 `replaceView`，但必须满足：

- 两个 mutation 都以各自 slot 的 revision/baseRevision 推进。
- 相同 `causationId`。
- Angular 任一 mutation 失败时两个 slot 都不 swap。
- `FrameApplied` 只能在两者提交后回传。

## 当前证据

- 内部 operation：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:150`
- operation 被丢弃并改发 replaceView：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:560`
- V2 mutation 目前 flatten V1 mutation：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/interaction_frame.rs:613`
- 当前 typed patch 都使用 `ReplaceView`：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/interaction_frame.rs:1267`
