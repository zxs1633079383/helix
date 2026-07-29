# Verification Procedure

## 六级证据模型

本规格的证据按下表逐级登记，低等级通过不能借给高等级使用。每条验证记录必须同时写明
`gate_id`、等级、命令、结果、产物路径、时间窗口和未覆盖边界。

| 等级 | 名称 | 本规格中的含义 | 允许证明 | 不允许冒充 |
|---|---|---|---|---|
| E1 | deterministic | 相同 Tick、注入时间、ID 与端口响应经 `ImModule::handle` 重放，完整 Effect 序列和生成文件 byte/hash 相同 | sans-IO 状态机确定性、重复生成稳定性 | 持久化、真实网络、真实客户端 |
| E2 | contract | Rust 合同测试验证 `event + data`、schema、Gate 集合、字段 lineage 和禁用字段 | MessageV3 结构与单模块业务合同 | 驱动执行、数据库读回、Go/WS |
| E3 | integration | 测试驱动执行 Persist/PortReply/Emit，且从临时 SQLite 读回后对账 | 写后释放、持久化原子性、模块与 driver 接缝 | 线上 Go、真实认证和真实 WS |
| E4 | runtime | 单 persona 通过真实宿主、真实 Go、真实 WS 和真实 tenant 完成同一 Gate | 单账号产品链可达与本地恢复 | 第二账号观察结果 |
| E5 | dual-user | Actor/Observer 使用隔离身份和独立本地库，在同一时间窗口完成双视角断言 | sender/receiver 或 persona-local 传播边界 | canonical 产物可复现与跨端一致 |
| E6 | canonical | `ScenarioRunner -> ImModule::handle -> Effect::Emit` 生成带 provenance 的唯一 Helix 产物，并由消费者 byte-exact 镜像 | 可审计的跨端唯一合同 | 未实际同步的客户端仓库已通过 |

当前版本只生成 Helix `.v1` 产物，不同步 `cses-client` 或 Mobile，因此即使 E1-E3
全部通过，消费者同步状态仍必须保持 `NOT_SYNCED_BY_SCOPE`。E4-E6 只有在相应命令和产物
真实存在时才能升级；源码入口、fixture 或历史 WDIO 文件只能登记为
`SOURCE_PRESENT / NOT_RUN`。

## 结果状态与失败分类

- `PASS`: 本等级的命令、断言和产物全部完成，且没有借用其他等级证据。
- `FAIL`: 命令已执行且业务断言失败；保留原始输出，不用摘要覆盖。
- `BLOCKED`: 所需 Go、认证、WS、设备、账号或消费者同步前置不存在。
- `NOT_RUN`: 只发现源码、fixture 或测试入口，当前证据窗口未执行。
- `NOT_APPLICABLE`: 业务合同明确为 persona-local，receiver 为空是断言而不是缺测。

失败至少归入 `DETERMINISM`、`SCHEMA`、`PERSISTENCE`、`PROTOCOL`、`RUNTIME`、
`DUAL_USER`、`CANONICAL` 或 `ENVIRONMENT`；不得把环境阻塞改写为业务通过。

## P0 公共基础执行记录

| 证据 | 等级 | 结果 | 原始输出 | 边界 |
|---|---|---|---|---|
| 三份合同测试首次编译 | E2 contract | `EXPECTED_RED` | `evidence/p0-contract-red.txt` | 缺少 `event/*`、PersistOk 编排与 artifact support，证明测试先于实现 |
| registry 单独编译 | E2 contract | `FAIL -> FIXED` | `evidence/p0-gate-registry-red.txt` | `DomainEventBytes` 不支持 `PartialEq`；改为上下文持有 owned bytes，未修改 core 合同 |
| event contract | E1/E2 | `PASS 3/3` | `evidence/p0-contract-focused.txt` | 证明 event/data-only、matching PersistOk 一次释放、PersistErr fail closed；未执行真实 SQLite |
| artifact schema | E1/E2 | `PASS 5/5` | `evidence/p0-contract-focused.txt` | 证明四类 schema、byte/hash 稳定和敏感字段拒绝；未生成任一业务 Gate 产物 |
| Gate registry | E2 | `EXPECTED_RED 1/2` | `evidence/p0-contract-focused.txt` | domain/package 已通过；30 个 Gate target 尚未逐 Gate 创建，明确留给 T019 收敛 |
| 静态迁移护栏 | E2 | `PASS` | `check-message-v3-cutover.mjs --self-test` | 30 Gate、owner、七类遗留引用和单活动项通过；消费者仍为 `NOT_SYNCED_BY_SCOPE` |

P0 当前最高连续等级为 E2。`message_v3_gate_registry` 的缺失 target 是计划内 RED，不创建空
测试或占位 JSON；因此公共事件与 artifact 基础可进入 Gate 纵切，但 30 Gate 分母仍不能宣称
Green。E3 integration、E4 runtime、E5 dual-user 与 E6 canonical 均为 `NOT_RUN`。

## P1 基线 Gate 执行记录

| Gate | 证据 | 等级 | 结果 | 原始输出与产物 | 边界 |
|---|---|---|---|---|---|
| G-15a | owner/invitee 独立 `ImModule::handle` 回放、channel/member 原子提交、PersistOk 后双事件 | E1/E2 | `PASS 2/2` | `evidence/p1-baseline-focused.txt`、`.v1/gates/G-15a/generated/` | 双 persona 是确定性本地回放，不是双账号真实链 |
| G-01 | sender/receiver 独立回放、message/channel/member/cursor 原子写、复合键读回后双事件 | E1/E2 | `PASS 2/2` | `evidence/p1-baseline-focused.txt`、`.v1/gates/G-01/generated/` | receiver 基线由测试输入注入，不证明真实 Go 广播 |
| G-01 storage | `ScopedGuardedBump` 与 `ScopedGet` 真 SQLite 复合主键隔离 | E3 integration | `PASS 1/1` | `evidence/p1-baseline-focused.txt` | 证明 HostStorage SQL 与读回，不证明 Web/FFI 产品运行态 |
| shared ports | core/native/FFI/Web check、core test、Web wasm build、core 零运行时依赖 | E2/E3 | `PASS` | `evidence/p1-baseline-focused.txt` | 仅 Rust 编译与驱动接缝，不替代 Gate runtime |
| cutover | 30 Gate 集合、迁移 owner 禁用词与负例自测 | E2 contract | `PASS / generated PARTIAL` | `evidence/p1-baseline-focused.txt` | 当前仅 G-15a/G-01 有 generated，剩余 28 Gate 明确未生成 |

G-15a 与 G-01 已在 migration manifest 标为 `MIGRATED`。两者当前最高连续等级分别为 E2 与
E3；E4 runtime、E5 真实 dual-user、跨端 mirror 后的 E6 canonical 均为 `NOT_RUN`，
消费者同步继续为 `NOT_SYNCED_BY_SCOPE`。

## 当前来源与缺口

| 证据面 | 当前来源 | 当前状态 | 不能证明 |
|---|---|---|---|
| Helix 历史 producer/test | `crates/helix-im/tests/g*_*.rs` 与既有 support scenario | `SOURCE_PRESENT / NOT_RUN` | MessageV3 event-only、旧链物理删除 |
| cses-client WDIO mock | `tests/e2e/wdio/specs` 的 mock/suite 场景 | `SOURCE_PRESENT / NOT_RUN` | Native、真实 Go/WS、双账号 |
| cses-client single-real | 同目录 `*-real.e2e.ts` | `SOURCE_PRESENT / NOT_RUN` | Observer 独立本地库与双视角 |
| cses-client dual-real | 同目录 `*-dual-real.e2e.ts` | `SOURCE_PRESENT / NOT_RUN` | 当前 Helix HEAD 或本轮 `.v1` canonical |
| receiver 明确断言 | G-01/G-04/G-15a/G-16/G-16b 等现有角色场景 | `PARTIAL` | G-15 Observer thread、G-16c 简介/公告等未覆盖字段 |
| persona-local | G-08/G-09/G-10/G-10a/G-10b/G-11/G-11a/G-13 | `INFERRED + PARTIAL_SOURCE` | receiver 为空的完整 production replay |

缺失 receiver 断言继续标 `MISSING`；从业务不变量补出的行为继续标 `INFERRED`。两者只有在
对应 Gate 的真实 Rust replay 和 persona artifact 落地后才能升级为 E2，不能由 WDIO 文件存在、
历史 Frame fixture 或人工 `.v1` sample 替代。

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
- **V-020**: `node
  specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs
  --require-generated` 必须证明 `.v1/gates` 目录与冻结 30 Gate missing/extra 均为 0；
  实现中默认模式只允许 missing，不允许任何额外 Gate。

## Evidence Rules

实现阶段的命令原始输出必须保存到 `evidence/`，并通过
`loop_engine.py record-evidence` 登记；失败输出不得用总结性文字覆盖。每个 Gate 的
最高等级取该 Gate 实际通过的最高连续等级：E2 失败时不能登记 E3，E4 阻塞时不得影响
已真实通过的 E1-E3，但总交付必须明确为 `PARTIAL` 或 `BLOCKED`。静态源码、Mock、
HTTP SUCCESS、fixture、build、WS 帧、SQLite 行、DOM 与双用户观察均是不同证据边界。
