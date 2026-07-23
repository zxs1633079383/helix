# G01 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

文本发送以 opaque `message.create` binding 进入，pending PersistOk 后显示 Sending，HTTP 仅受理，matching WS/sync + canonical PersistOk 后同一行收敛 Sent。

## 影响文件

- Helix：`src/module/action.rs`、`src/send/{flow,pending,reconcile}.rs`、`surface_composer.rs`、`tests/{send_test,f1_send_http_reconcile_test}.rs`
- cses-client：composer、timeline/runtime、projection ingress 与对应测试
- 数据：PG `post/channel/channel_member/channel_event`；SQLite `message/pending_send`

## 实施顺序

1. RED：binding、temporary persist、HTTP 2xx、duplicate/late echo、PersistErr。
2. Helix：temporary→canonical identity 状态机和 post-commit V2 Frame。
3. Client：V2-only timeline/composer apply，commit 后 clear draft。
4. Evidence：双用户、PG/SQLite identity、Frame/DOM 同 causation。

## 完成闸门

一次发送只产生一条 canonical 行；observer 可见；V1/raw 零命中。
