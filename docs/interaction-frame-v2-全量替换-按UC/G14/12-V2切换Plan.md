# G14/G14a V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

单条 post_read 与 template_received 分离 action/状态，按 author/reader viewer 更新 receipts、timeline 和 thread。

## 影响文件

- Helix：post-read outbound/WS、receipt/timeline/thread projectors、`tests/g14_receipt_template_interaction_test.rs`
- cses-client：read-receipt/template-content 与 G14 tests
- 数据：PG `post/channel_member/channel_event`；SQLite `message_receipt`

## 实施顺序

RED 覆盖重复、跨 post、非 viewer、语义混写、PersistErr；实现分离 normalize/persist/V2；客户端呈现独立状态；双视角证据。

## 完成闸门

read 与 received 不混写，receipt 幂等。
