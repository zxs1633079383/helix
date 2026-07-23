# G13 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

频道可见/已读 action 推进单调 read boundary，并原子更新会话未读、全局摘要和时间线。

## 影响文件

- Helix：channel/posts read outbound/WS、dialog/unread/timeline projectors、`tests/g13_channel_read_interaction_test.rs`
- cses-client：read-reporting、dialog-list 与 G13 tests
- 数据：PG `post/channel_member/channel/channel_event`；SQLite `channel/message_receipt`

## 实施顺序

RED 覆盖旧/重复 boundary、节流、viewer 隔离、PersistErr；实现 authority persist/V2 compose；客户端不乐观清零；三 Surface/双用户证据。

## 完成闸门

boundary 单调、未读一致、无请求风暴。
