# GS04 V2-only 切换 Spec

**Gate**: GS04 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

用户进入本地未知频道时，Helix 对单频道执行增量加载并建立可操作的频道头、composer 与时间线；未获权限或持久化失败时不得制造可发送 UI。

## 合同与影响面

| 维度 | GS04 合同 |
|---|---|
| 输入/命令 | `HostObservation` → `im_channel_load_increment_by_channel_id` |
| Effect/Persist | `http_read`、`storage_upsert`、`projection`；`channel`、`channel_member`、`message` |
| Go | `/channel/load/incrementByChannelId`；后续可有 `increment_channel` |
| PG | `channel`、`channel_member`、`post` |
| SQLite | `channel`、`channel_member`、`message` |
| Surface/VM | channel-summary、composer、timeline；ChannelSummaryVm + ComposerVm + TimelineWindowVm |
| Angular | message-ui-header、message-ui-composer、message-surface-runtime-view |

## 专属证据清单

- [ ] 未知 channelId 只加载目标频道，返回 viewer membership 与权限。
- [ ] PG 频道/成员/消息与 SQLite write-set 使用同一 channelId 对账。
- [ ] 三个 Surface 在 PersistOk 后形成一致 V2 snapshot，不出现有输入框但无权限的中间态。
- [ ] forbidden/not-found/empty/PersistErr 保持可解释状态，零伪成功 Frame。
- [ ] DOM 频道头、composer enabled 状态和时间线身份一致。

## 验收标准

陌生频道加载后可按权限操作，失败时 fail-closed；纯 V2 首轮与删除后回归均通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
