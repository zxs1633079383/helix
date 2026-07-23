# G10/G10a V2-only 切换 Spec

**Gate**: G10、G10a | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

用户 attach 频道时获得首屏有界窗口，上拉时向前补窗；本地数据完整时零网络，缺口才回源，提交后保持滚动锚点。

## 合同与影响面

| 维度 | G10 合同 |
|---|---|
| 输入/命令 | HostObservation / `ActionBinding(load_older)` → query/load context |
| Effect/Persist | local_scan、http_read、storage_upsert、projection；`message` |
| Go | 条件 `/posts/getLatestPost`；无 WS |
| PG | `post`、`channel` |
| SQLite | `message` |
| Surface/VM | `message.timeline`；TimelineWindowVm |
| Angular | message-surface-runtime-view、scroll-anchor-store |

## 专属证据清单

- [ ] Complete 分支零 HTTP；Incomplete 分支只请求声明缺口。
- [ ] 窗口大小、边界、顺序与去重可证伪，PG/SQLite identity 一致。
- [ ] snapshot/patch 的 baseRevision 连续，旧响应不能覆盖新窗口。
- [ ] DOM 补窗后滚动锚点保持，无跳顶、重复行或空白。
- [ ] empty、end-of-history、并发上拉、超时和 PersistErr 保留上一窗口。

## 验收标准

首屏和补窗有界、确定、无重复，滚动稳定；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
