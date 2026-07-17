# NEED: Command API runtime alignment

状态：`CONTRACT_DRIFT`。本文件记录 Strict Matrix 已声明、但当前
`helix-im` outbound/runtime 尚未以相同字段或投影兑现的项目。它是
下游 PC/Mobile 的 fail-closed 边界，不是兼容层授权，也不表示任一
HTTP、WS、storage 或 projection 链路已验收。

## 字段漂移

| Gate | Canonical Matrix input | 当前 Helix runtime input | 处理要求 |
| --- | --- | --- | --- |
| G-18a `im_announcement_read` | `announcement_id` | `post_id`, `channel_id` | 在 Matrix 与 outbound 之间选定一个 canonical body 后，同步改合同、测试和下游。 |
| G-20 `im_channel_member_change` | `channel_id`, `join_users`, `leave_users` | `channel_id`, `join_user_ids`, `leave_user_ids`，并可依赖 `team_id`、`self_id` | 不允许 PC/Mobile 用 host alias 翻译；应由 Helix 收敛 canonical input。 |
| G-21 `im_channel_member_snapshot` | `channel_id`, `req_id`，可选 revision/digest | `channel_id`, `start_time`, `end_time` | 补齐正式快照请求字段或修订 Gate，不可把 UI 侧时间推导伪装为合同实现。 |
| G-26c `im_team_upsert` | `team_id`, `display_name` | 还要求 `self_id`、`member_ids` | 将业务 body 成形所需字段明确进入 Matrix，或在 Helix 由已持久化身份补齐。 |
| G-26d `im_team_quit` | `team_id` | 还要求 `user_id` | 同上；下游不能猜测或注入用户身份。 |

## 缺失投影

Matrix 目前声明但 `crates/helix-im/src` 没有同名 emit 的 projection：

- `im:announcement:updated`
- `im:module:read`
- `im:company-group:updated`
- `im:system-notice:updated`

这些 Gate 可以保留 `declared`，但不得被 contract 文档、PC/Mobile UI 或
历史 evidence 晋升为 runtime-green。实现时必须同时补投影 schema、负向
测试、持久化/WS 证据和下游 renderer consumption。

## 下游规则

- UC1-16 只消费 `command-api.v1.json` 中的 canonical command/input；G03
  媒体真实链按当前发布决策不作为本轮阻塞项，不能以 stub 伪造完成。
- 对表内 Gate，PC/Mobile 可以保留入口或显示明确不可用状态，但不得在
  transport 壳层改名、补字段或制作成功 mock。
- 本文件关闭前必须由 Matrix、`client_api::build_command`、outbound tests、
  projection schema 和两个下游生成门禁共同证明同一 raw matrix SHA。
