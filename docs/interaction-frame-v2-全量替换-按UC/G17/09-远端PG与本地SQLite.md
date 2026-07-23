# G17 远端 PostgreSQL 与本地 SQLite

## 数据所有权

| 层 | 所有内容 | 不应拥有 |
|---|---|---|
| 远端 PostgreSQL | channel、member、post、topic、event cursor 的权威事实 | Angular section/order |
| Go application | 权限、事务、WS audience 与权威事件 | viewer UI patch |
| 本地 SQLite | 可恢复的 channel/member/message/cursor 副本 | 第二套业务真源 |
| Helix projector state | render-ready row、section order、per-slot revision/epoch | 远端事务权威 |
| Angular slot state | 已验证 VM 和短生命周期 UI 状态 | 业务排序、未读推导 |

## V1 当前读写

初始 `im_query_dialog_list` 使用：

```text
Scan(channel ORDER BY is_top,last_post_at,created_at DESC LIMIT)
```

该路径无 `channel_id` 过滤，默认 500、最大 2000，属于低频 snapshot。在线事件进入已有 `DialogListFrameState` 后，不再为每一行变化执行同样的全表 scan。

V1 的问题是：即使存储读取已按需，正式 Frame 仍重新构造和发送整个缓存 view。

## V2 目标读写

1. 冷启动、明确 refresh、revision gap 和 epoch 重建允许 bounded snapshot scan。
2. 在线事件先使用当前 projector state；事实完整时零额外 DB 读取。
3. 陌生或缺字段频道只执行 keyed `channelId` hydration。
4. targeted hydration 写入 SQLite 成功后，只有已有 snapshot 才发布 delta；否则等待 snapshot。
5. delta 不写一份额外“UI patch 表”；恢复仍基于业务副本与 cursor 重建。
6. pagination window token 属于 Surface state，不进入远端业务表。

## 一致性与恢复

- SQLite persist 成功不等于 DOM 完成。
- PortReply 成功后仍需 Helix Frame、Tauri transport、Angular atomic apply 和 `FrameApplied`。
- gap 恢复使用 snapshot 覆盖旧 window，不重放无法验证 anchor 的 delta。
- refresh/late-entry 重建结果必须与连续增量后的最终 `DialogListVm` 相同。

## 复杂度

| 路径 | 时间 | payload/空间 |
|---|---|---|
| 初始 snapshot | `O(n)` scan + projection | `O(n)` |
| 在线 keyed upsert | 平均 `O(1)` row lookup + 当前 order 调整 | wire `O(k)` |
| move | 当前 Vec 最坏 `O(n)`；未来可换 order index | wire `O(1)` operation |
| Angular atomic stage | `O(k + shifted)` | 受影响 section copy-on-write |

G17 首要目标是消除在线 wire 的 `O(n)`；是否把 Helix Vec 排序进一步改成树结构，应由基准数据证明后再决定。

## 当前源码证据

- snapshot scan 边界：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/query/core.rs:228`
- 单频道 hydration patch：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/query/core.rs:282`
- 内存 row map 与 order vec：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:176`
- 当前 reposition 最坏 `O(n)`：`/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/dialog_frame.rs:736`
