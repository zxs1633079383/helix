# GS04 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

未知频道先 hydrate 并持久化 channel/member/message，再原子建立 summary/composer/timeline；PersistOk 前不得签发可发送 binding。

## 影响文件

- Helix：`src/channel/gate/flow.rs`、`src/sync/flow.rs`、`surface_composer.rs`、`tests/uc45_unknown_channel_post_test.rs`
- cses-client：`message-surface.facade.ts`、composer/header/runtime-view 与恢复 DOM 测试
- 数据：PG `channel/channel_member/post`；SQLite `channel/channel_member/message`

## 实施顺序

1. RED：unknown/forbidden/not-found/empty/PersistErr 与 pre-commit no-send。
2. Helix：scoped hydrate、atomic write-set、post-commit interaction bundle。
3. Client：三个 Surface 同 epoch/revision apply，按 binding 启用 composer。
4. Evidence：频道/成员/消息 identity 与 UI 权限同 causation。

## 完成闸门

三 Surface 一致出现，失败 fail-closed，无可发送中间态。
