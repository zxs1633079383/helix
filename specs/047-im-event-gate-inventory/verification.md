# Verification Procedure

## Required Runs

- **V-001**: `jq '[.uc_cases[]] | {total:length, active:(map(select(.lifecycle ==
  "active")) | length), excluded:(map(select(.lifecycle == "excluded")) | length)}'
  crates/helix-im/contracts/strict-matrix.v1.json`，预期精确输出
  `{"total":50,"active":47,"excluded":3}`。
- **V-002**: 从 `spec.md` 提取本轮截图 Gate ID，与冻结的 30 Gate 清单做集合对账；
  预期缺失 0、额外 0、重复 0。
- **V-003**: 对本轮 30 个 Gate 检查 cses-client 的 `mock`、`single-real`、
  `dual-real` WDIO 源入口；预期 90 个 Gate-lane 组合均有来源。G-01 mock
  由 suite 与 sent/failed/WS-dialog 场景组成，不要求虚构一个统一
  `g01-mock.e2e.ts`。该检查只证明源码存在。
- **V-004**: `rg -n 'placeholder|\[Replace|TODO|TBD'
  specs/047-im-event-gate-inventory`，预期无未解析模板占位符。
- **V-005**: `python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance`，
  预期 mandatory `r1_spec` 从 blocked 修复为 passed，并进入等待 plan 的状态。
- **V-006**: `git diff --check`，预期无空白错误。
- **V-007**: `node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs`，
  预期 `.v1` 生成 30 个 Gate 目录、120 个声明合同文件、30 个捕获状态文件和
  60 个结构样例文件及 30 个 `captures/` 槽位目录；连续生成两次的全树 SHA-256 相同。
- **V-008**: `git check-ignore -v
  specs/047-im-event-gate-inventory/.v1/manifest.json` 必须命中仓库
  `**/.v1/` 规则；当前 manifest 必须如实报告 90 个原始捕获槽位中
  0 个存在、90 个缺失、总判定 `INCOMPLETE`。
- **V-009**: `.v1` 的三类捕获必须精确为 `inbound/effects/emit`；`outbound-contract.json`
  中每个 channel 都必须声明 `core_output=Effect::Emit` 和源码位置。
- **V-010**: `rg -ni 'surface|interactionframe|im:interaction:frame|\bdom\b'
  specs/047-im-event-gate-inventory/.v1`，预期无匹配；UI 层不得进入本地 v1 业务出站合同。
  `ws/frame.rs` 属于真实 transport parser，不在该禁用层范围。
- **V-011**: 30 个 Gate 必须各有 `inbound-sample.json` 和 `outbound-sample.json`；
  共 60 份样例均须 `runtime_capture=false`。存在 `TARGET_REQUIRED` channel 或
  `inferred_gaps` 的 Gate 标记为 `SOURCE_DERIVED_TARGET_SAMPLE`，其余标记为
  `SOURCE_DERIVED_REVIEW_SAMPLE`，二者都不得冒充 runtime capture。
- **V-012**: 30 份 outbound 样例都必须同时包含 `sender_samples` 与
  `receiver_samples`，且每项只能含 `event` 与 `data`。禁止旧 `samples`、
  `sender_failure_samples`、`actionCapabilities`、`channelCapabilities`、snake/camel
  重复别名、`unreadDelta`、`effect_variant` 和 adapter view；两视角 event 并集必须
  精确覆盖对应 `outbound-contract.json.channels`。
- **V-013**: persona-local Gate `G-S3/G-08/G-09/G-10/G-10a/G-10b/G-11/G-11a/G-13`
  的 `receiver_samples` 必须为空。G14 必须只向 reader/sender 给
  `im:post:read`、向 author/receiver 给 `im:post:readers`；G16b 必须把 remaining
  roster 与 leaver left 分到不同视角。
- **V-014**: G01/G02/G03a/G05/G10c/G10d 的 sender/receiver 消息成功样例必须带
  写后 `im:channel:update`。G01 的 sender `unreadCount` 必须为 0，receiver 基线为 1；
  两者 `lastPost` 必须含同一消息的 `id/userId/message/createAt`，且 `createAt` 与
  `im:post:received` 精确一致。
- **V-015**: G03a inbound 必须含
  `im_retry_send{message_id=<failed temporaryId>}`；sender outbound 必须按
  `im:post:send-failed` → `im:post:sending` → `im:post:received` →
  `im:channel:update` 展示 retry 成功路径，failed 与 retry-sending 必须复用同一
  `temporaryId`。receiver 不得收到 retry command、failed 或 retry-sending，只在成功后
  收到 `im:post:received` 与自己的 `im:channel:update`。不得虚构生产中不存在的
  `im:post:retrying`。
- **V-016**: G04 inbound 必须展示 `posts_update` 中 `revoke=true`、鉴权 actor 的
  `system_event`，以及仅 lastPost 分支出现的 `update_channel.data.lastPost` JSON 字符串。
  outbound sender/receiver 必须各只有 `im:post:revoke` 与 `im:channel:update`：
  两边 post id 相同且 `revoke=true`；raw post data 必须全部映射到本地 `message` 列，
  不得包含 `actorName/systemEvent/displayText` 或 viewer 文案；channel update 必须直接映射
  到各自 `channel_member.last_post/unread_count`，撤回不增加未读绝对值。
- **V-017**: 30 份 inbound 样例必须含 `remote_persistence_basis`，30 份 outbound
  样例必须同时含 sender/receiver `local_persistence_basis` 与 `field_lineage`。所有引用的
  表和列都必须存在于 `contracts/storage-schema-lock.v1.json`；schema lock SHA-256 必须一致。
- **V-018**: storage schema lock 必须固定远端
  `10.200.9.168/32:5432/mattermost-imv2/public` 与本地 444/999 两个 tenant DB 地址，
  且只保留 schema/key shape，不包含真实业务行、密码或 token。
- **V-019**: 每个 outbound event 的顶层 data 字段必须恰好落入
  `direct_local_sources` 或 `non_storage_or_derived_fields` 之一。后者只表示查询/分页、
  correlation、角色派生或待确认字段，不能登记为持久事实。

## Evidence Rules

本轮属于 specify 阶段，不运行 Cargo、WDIO 或真实链，也不把静态检查登记成运行态
Green。进入 plan/implement 后，命令原始输出必须保存到 `evidence/`，并通过
`loop_engine.py record-evidence` 登记；失败输出不得用总结性文字覆盖。
