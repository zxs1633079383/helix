# G02 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

复用 G01 identity/authority 状态机，仅扩展 DOCUMENT typed content；未知 props/version 拒绝，不允许 Angular raw props reducer。

## 影响文件

- Helix：`interaction_frame.rs`、send flow/content normalize、contract/send tests
- cses-client：`surface-contract.*`、`message-document-content.component.ts` 及 content/runtime tests
- 数据：PG `post/channel/channel_member/channel_event`；SQLite `message/pending_send`

## 实施顺序

1. RED：typed document、unknown version、missing field、upload/HTTP/Persist failure。
2. Helix：权威 props → typed VM，复用 pending/canonical flow。
3. Client：typed component 机械渲染，零 raw JSON business branch。
4. Evidence：PG props 摘要、SQLite typed row、双用户 DOM，不记录敏感正文。

## 完成闸门

文档内容 typed、唯一收敛、失败可恢复，V1/raw props 零命中。
