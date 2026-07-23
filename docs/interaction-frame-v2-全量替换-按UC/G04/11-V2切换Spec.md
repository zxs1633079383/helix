# G04/G04a V2-only 切换 Spec

**Gate**: G04、G04a | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

作者或管理员在线撤回消息，或用户离线恢复 type-3 事实；两条入口必须归一为同一撤回态、系统提示和失效 ActionBinding。

## 合同与影响面

| 维度 | G04 合同 |
|---|---|
| 输入/命令 | `ActionBinding(revoke)` / Inbound → `im_revoke` |
| Effect/Persist | http_write、storage_upsert、projection、sync_request；`message` + cursor |
| Go | `/posts/revoke`；`posts_update`、`post_update`、`update_channel` |
| PG | `post`、`channel`、`channel_member`、`channel_event` |
| SQLite | `message`、`channel_event_cursor` |
| Surface/VM | timeline、thread-detail、left-dialog-list；完整撤回 VM |
| Angular | message-ui-action-menu、message-surface-runtime-view、thread drawer |

## 专属证据清单

- [ ] 在线路径恰好一个 revoke HTTP；离线 G04a 零 HTTP。
- [ ] 两条路径的 PG authority/SQLite write-set/V2 Frame 结果语义相同。
- [ ] 原消息、thread 引用、会话预览和旧 actionRef 同步失效。
- [ ] 无权限、重复、乱序、late fact、PersistErr 不推进错误 revision。
- [ ] actor/observer 撤回文案按 viewer 正确，截图与 DOM 同一运行。

## 验收标准

在线和离线撤回最终态一致，无残留可操作旧消息；纯 V2 两轮回归通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
