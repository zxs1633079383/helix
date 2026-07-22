# G15 Implementation Handoff

## 状态

`PARTIAL - dedicated topic invariants ready; shared action integration pending`

## 已完成

- `TopicListProjector` 新增 `topicCount >= visible rows` 与 topicChannelId 唯一性校验，失败整视图拒绝。
- focused UI contract 证明现有 CreateTopic 只输出 topic intent，不拥有 G15a createChannel。
- G15a 继续只引用 `003-g15a-create-channel-pc`，本组未重复建群生产实现。

## 证据

- Angular：`evidence/angular-focused.log` 末段 retry 1/1 PASS。
- Rust：focused final 尚待 root 在 shared 编译修复后聚合重跑。

## 未决

- shared `topicCreate -> im_make_topic` binding、corr、authoritative post/channel/member facts、PersistAtomic 与 topic/timeline/summary Frame。
- 独立话题窗口真实消费由 017 验收。
