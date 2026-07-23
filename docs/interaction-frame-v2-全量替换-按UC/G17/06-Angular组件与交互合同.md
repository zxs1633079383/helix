# G17 Angular 组件与交互合同

## 生命周期边界

G17 没有直接按钮和新增业务 handler。它是 observation-driven projection，Angular 只消费 Helix 给出的行操作。当前没有 exact Gate，因此本合同状态为 `design-blocked`。

| 字段 | 内容 |
|---|---|
| 组件文件 | `src/pages/message-v4/components/message-surface-runtime-view.component.ts`、`surface/message-ui-surface-dialog-list.component.ts` |
| Surface adapter | `message.left-dialog-list` adapter、`message.unread-summary` adapter |
| 消费 VM | `DialogListVm`、`UnreadSummaryVm` |
| 用户 output | 无 |
| ActionBinding | 无；不得伪造 `dialog.delta` action |
| 接收 Frame | `ProjectionBusService -> ProjectionStore -> SurfaceRuntime` |
| 当前 patch | `dialogListPatch.patch.kind=replaceView` |
| 目标 patch | `dialogListPatch.patch.kind=rowDelta` |
| 本地 UI 状态 | 搜索词、展开态、滚动位置；不得拥有业务排序与计数 |
| Operation visibility | Frame 本身为当前 viewer 的 `self-only` user scope；权威变化可为 `multi-party` |

## 目标 wire 类型

以下只是 G17 提案，必须在 Gate 登记后才是正式合同：

```ts
interface DialogListDeltaPatch {
    readonly kind: 'rowDelta';
    readonly operations: readonly DialogListDeltaOperation[];
    readonly sectionPages: readonly SectionPagePatch[];
}

type DialogListDeltaOperation =
    | {
          readonly kind: 'upsertRow';
          readonly channelId: string;
          readonly row?: DialogRowVm;
          readonly set: DialogRowSet;
          readonly clear: readonly DialogRowClearKey[];
      }
    | { readonly kind: 'removeRow'; readonly channelId: string }
    | {
          readonly kind: 'moveRow';
          readonly channelId: string;
          readonly section: 'pinned' | 'normal' | 'closed';
          readonly position:
              | { readonly kind: 'start' }
              | { readonly kind: 'before'; readonly channelId: string }
              | { readonly kind: 'after'; readonly channelId: string };
      };

interface SectionPagePatch {
    readonly section: 'pinned' | 'normal' | 'closed';
    readonly expectedWindowToken: string;
    readonly page: WindowPageVm;
}
```

## Codec 规则

1. V1 只接受 `replaceView`；V2 `rowDelta` 必须是新分支，禁止兼容猜测。
2. `operations` 必须非空且有上限；未知 kind、未知字段、重复冲突操作 fail-closed。
3. 新增 row 的 `upsertRow` 必须携带完整 `row`；已有 row 才允许只携带 `set/clear`。
   `DialogRowSet` 和 `DialogRowClearKey` 必须是显式白名单，不得允许修改 row identity、`channelId` 或 section/order。
4. `removeRow` 与同频道其他操作互斥。
5. `moveRow` 的 row 和 anchor 必须在 stage 后存在，且属于声明 section。
6. 每个受影响 section 必须携带匹配当前窗口的 `expectedWindowToken` 和新的 `page`。
7. `surfaceEpoch`、`baseRevision`、scope 任一不匹配，整帧进入 `resync-required`。

## Runtime 原子应用

当前 `stageMutation()` 最终调用 `viewForMutation()` 直接取得完整 replacement。G17 目标必须改为：

1. 从 ready slot 创建 copy-on-write stage。
2. 用 `Map<channelId, RowLocation>` 做平均 `O(1)` 身份查找。
3. 按 wire 顺序应用 operation，但不修改 live slot。
4. 校验最终 channel 唯一、section 正确、anchor 有效、page token 推进和 row schema。
5. dialog 与 paired unread 全部成功后一次 swap。
6. 最后执行 directive 并发送 `FrameApplied`。

复杂度不得写成虚假的严格 `O(k)`：数组中的 move 会移动一个区间，因此客户端实际为 `O(k + shifted)`；关键收益是 payload 不再为 `O(n)`。

## DOM 验收

- 未变化 row 的 `data-dialog-id`、节点 identity 和可见顺序保持稳定。
- 新增、关闭、置顶和更新 preview 只影响目标 row 及必要相邻顺序。
- paired unread 与 row badge 同一次 render turn 可见。
- gap 时不显示半份结果，列表进入恢复态。
- refresh/late-entry 用 snapshot 得到与连续 delta 相同的 DOM。

## 当前证据

- V1 类型明确要求完整 replacement：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:421`
- codec 只接受 `isReplaceViewPatch`：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:365`
- runtime 已有整帧 staging 与 revision-gap 框架：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts:274`
- 旧的独立 dialog frame 类型虽包含 `upsert/remove/move`，但不是当前 SurfaceRuntime 正式 wire：`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/left-dialog-list-frame.types.ts:77`
