# GS03 V2-only 切换 Spec

**Gate**: GS03 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

在线用户收到 `pong.gaps` 或 hash mismatch 后，只对确有差异的频道执行有界补偿；同步状态可见，但最后一致业务画面不得被清空。

## 合同与影响面

| 维度 | GS03 合同 |
|---|---|
| 输入/命令 | `Inbound + Timer` → `im_sync_channels` |
| Effect/Persist | `sync_request`、`projection`；PersistAtomic：`channel_event_cursor`、`message` |
| Go | `pong.data.gaps/hashMismatch` + `/channel/sync/notify` |
| PG | `channel_event`、`post`、`channel` |
| SQLite | `channel_event_cursor`、`message` |
| Surface/VM | left-dialog-list、unread-summary、timeline；InteractionRuntimeVm + 受影响 VM |
| Angular | message-ui-state、message-surface-runtime-view |

## 专属证据清单

- [ ] 普通 pong 不触发全频道扫描；只有 gap/hash mismatch 频道进入补偿队列。
- [ ] 同频道并发触发合并，队列和重试有界，不形成自激请求风暴。
- [ ] PG head、请求 fromSeq、SQLite cursor 与补偿后 Frame 可按 correlation 串联。
- [ ] no_change、重复 gap、断网重试和 PersistErr 不破坏当前 Surface。
- [ ] 同步状态结束后无残留 loading，时间线和未读摘要保持一致。

## 验收标准

目标 gap 被有界追平且非 gap 频道零请求；无同步风暴、无 UI 回退，纯 V2 两轮回归通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
