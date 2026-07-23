# G11 V2-only 切换 Spec

**Gate**: G11 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

用户请求定位指定消息时，Helix 先准备包含目标的完整窗口；只有窗口整帧提交后才执行 reveal/focus directive。

## 合同与影响面

| 维度 | G11 合同 |
|---|---|
| 输入/命令 | `ActionBinding(locate)` → `im_get_posts` |
| Effect/Persist | http_read、storage_upsert、projection；`message` |
| Go | 本地优先；必要时 `/posts/get` 或 postContext；无 WS |
| PG | `post` |
| SQLite | `message` |
| Surface/VM | timeline；TimelineWindowVm + RevealMessage directive |
| Angular | message-surface-runtime-view、scroll-anchor-store |

## 专属证据清单

- [ ] 本地命中零 HTTP；缺失才回源目标上下文。
- [ ] PG/SQLite/Frame 都包含目标 postId 和有界上下文。
- [ ] directive 在 Frame 原子提交后执行，不能先滚动后补数据。
- [ ] not-found、forbidden、stale actionRef 和 PersistErr 不移动当前视图。
- [ ] 目标 DOM 可见、聚焦/高亮正确且无永久动画残留。

## 验收标准

定位只揭示已提交目标窗口，失败不扰动当前视图；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
