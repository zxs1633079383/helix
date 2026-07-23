# G07 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

urgent 与 urgent_confirm 使用不同 binding/authority，按 viewer 更新消息、参与者回执和未读摘要。

## 影响文件

- Helix：urgent outbound/render-ready、receipt projector、`tests/g07_urgent_interaction_test.rs`
- cses-client：urgent/read-receipt 组件与 G07 tests
- 数据：PG `post/channel/channel_member/channel_event`；SQLite `message/message_receipt`

## 实施顺序

RED 覆盖非目标确认、重复确认、拒绝/PersistErr；实现分离 action 和 receipt compose；客户端按权限呈现按钮/人数；双用户证据。

## 完成闸门

目标列表、确认人数和 viewer 权限一致，V1/raw 零命中。
