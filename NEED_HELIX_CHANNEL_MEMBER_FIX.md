# NEED: Helix channel_member read/count 投影补齐

## 背景

本轮已修复 `im:channel:update-by-post` 的 Dialog 计数语义：`unreadCount`、`mentionCount`、`urgentCount` 只从 Helix storage 写后 `channel` 行读回，不再把事件 delta 当累计终值交给前端累加。

## 当前缺口

`readLastSeq` / `msgCount` 属于 `channel_member` 级别的读水位与计数字段，但 Helix 当前本地 schema 与写路径还没有完整承接 Go `channel_member` 的 `lastReadSeq`、`msgCount`、`msgCountAll` 等累计投影。

## 本轮处理

- 已从 render-ready `dialogPatch` 中移除 `readLastSeq: null` 与 `msgCount: null`，避免前端/mobile 误认为 Helix 已完成字段对齐。
- 当前 patch 中这些字段视为 absent/optional；消费者不得按非空累计字段读取。

## 后续期望

1. 在 Helix 本地 schema 中明确 `channel_member` 的 read/count 字段归属。
2. 补齐 send/read/sync 写路径，把 `lastReadSeq`、`msgCount`、`msgCountAll` 等字段按写后累计终值维护。
3. 等 storage 与投影链路闭环后，再把 `readLastSeq` / `msgCount` 加回 DialogPatch。
