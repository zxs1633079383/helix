# G15/G15a V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

topic/create-channel 各自完成权威 entity bundle 持久化，再建立 topic/list/summary/member 首批 Surface。

## 影响文件

- Helix：channel-create/reply/topic projectors、`tests/{g15_topic_interaction_test,channel_create_http_reconcile_test}.rs`
- cses-client：create-topic/create-chat/runtime 与 G15 tests
- 数据：PG `channel/channel_member/channel_topic/post/channel_event`；SQLite `channel/channel_member/message/reply_tree`

## 实施顺序

RED 分别覆盖 topic/channel、重复请求、无效成员、HTTP-only、PersistErr；实现 entity bundle/V2 first surfaces；客户端 operation terminal；新 identity/双用户证据。

## 完成闸门

新实体唯一、成员正确、首批 Surface 原子可用。
