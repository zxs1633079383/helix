# GS02 V2-only 切换 Spec

**Gate**: GS02 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

重连用户收到 sync notify 后，从本地 cursor 有界补齐离线事件；缓存继续可见，只有消息与 cursor 原子提交成功后才更新会话、未读和时间线 Surface。

## 合同与影响面

| 维度 | GS02 合同 |
|---|---|
| 输入/命令 | `Inbound` → `im_sync_channels` |
| Effect/Persist | `sync_request`、`projection`；PersistAtomic：`channel_event_cursor`、`message` |
| Go | `/channel/sync/notify`，单批最多 500 cursors；并发 WS 不替代 sync response |
| PG | `channel_event`、`post`、`channel`、`channel_member` |
| SQLite | `channel_event_cursor`、`message` |
| Surface/VM | left-dialog-list、unread-summary、timeline；恢复运行态与完整受影响 VM |
| Angular | message-ui-state、message-surface-runtime-view |

## 专属证据清单

- [ ] 请求 fromSeq、PG event_seq、返回 nextSeq 与 SQLite cursor 连续且可关联。
- [ ] 满批继续拉取、mixed terminal 和 no_change 都有明确终止条件。
- [ ] 消息与 cursor 同一 PersistAtomic；PersistErr 时两者均不推进。
- [ ] 在线并发 WS 与离线 sync 去重，重复事实不产生第二次业务 mutation。
- [ ] 恢复中保留最后一致 DOM，完成后整帧切换，无闪空或半提交。

## 验收标准

所有目标频道补齐到权威 cursor，消息与 UI 一致且无重复；Frame 全为 V2，首轮纯 V2和删除 V1 后两轮均通过公共验收合同。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
