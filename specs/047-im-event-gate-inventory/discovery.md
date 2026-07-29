# Source Discovery

**Feature**: `specs/047-im-event-gate-inventory`
**Source Lock**: `240583184791269f39a7d18b5cae0f82d5baf54d`
**Implementation Scope**: 精确 30 Gate；`G-03` 与 `G-17` 至 `G-27` 均为
`OUT_OF_SCOPE`。本版本只生成 Helix `.v1` 产物，不同步 PC/Mobile 消费者。

**Evidence Baseline**: 当前已有源码、fixture 与 WDIO 入口仅登记为
`SOURCE_PRESENT / NOT_RUN`。MessageV3 的 deterministic、contract、integration、
runtime、dual-user、canonical 六级证据必须分别执行和登记。

## Facts *(mandatory)*

- **F-001**: 当前 Strict Matrix 的合同版本为 epoch 12，定义 50 个 Gate，其中 active 47、excluded 3；截图中的 Gate 不是当前 Matrix 全量分母。 | **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json:3`
- **F-002**: 截图列出的本轮分母为 30 个 Gate；逐项可在当前 Matrix 找到，但截图遗漏
  active 的 G-03，且未包含后续 G-17 至 G-27。 |
  **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json:48`,
  `crates/helix-im/contracts/strict-matrix.v1.json:52`,
  `crates/helix-im/contracts/strict-matrix.v1.json:77`
- **F-003**: 本轮 30 个 Gate 均存在 Helix producer/fixture 合同源；G-10b、G-10c、
  G-10d、G-11a 使用 timeline formal producer，其余第一批 Gate 有对应完整流程或
  canonical fixture producer。此事实只证明测试源存在，不证明本轮已执行。 |
  **Evidence**: `crates/helix-im/tests/g01_wdio_fixture_contract_test.rs:1`,
  `crates/helix-im/tests/g10d_wdio_fixture_contract_test.rs:1`,
  `crates/helix-im/tests/g11a_wdio_fixture_contract_test.rs:1`
- **F-004**: 本轮 30 个 Gate 均存在 WDIO mock、single-real、dual-real 场景源；
  dual-real 文件以两个隔离 profile 的 Actor/Observer 作为角色。源码存在不等于
  Mock、Native、Go、WS、双账号或真实应用可见状态已在本轮通过。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-dual-real.e2e.ts:49`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g16-dual-real.e2e.ts:16`
- **F-005**: G-03 当前仍是 active Gate，并存在 Helix durable media 合同与 WDIO
  dual-real 场景，因此不能因为截图遗漏而判定为退役或 excluded。 |
  **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json:620`,
  `crates/helix-im/tests/g03_media_durable_restart_contract_test.rs:1`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g03-dual-real.e2e.ts:1`
- **F-006**: Matrix 可继续提供 Gate ID、canonical command 和完成条件，但其中旧 UI
  投影不再作为本规格的业务出站；G-01、G-15、G-16c 必须分别回到当前 producer 的
  `im:post:*`、`im:channel:*` `Effect::Emit` 合同。 |
  **Evidence**: `crates/helix-im/contracts/strict-matrix.v1.json:481`,
  `crates/helix-im/contracts/strict-matrix.v1.json:1931`,
  `crates/helix-im/contracts/strict-matrix.v1.json:2128`
- **F-007**: 双账号场景并不都属于 sender/receiver 同态传播：G-08 明确要求定时草稿
  只对创建者可见；G-11 明确要求 Actor 定位不让 Observer 高亮；G-13 明确要求 Actor
  已读不改变 Observer 的未读投影。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g08-dual-real.e2e.ts:29`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g11-dual-real.e2e.ts:24`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g13-dual-real.e2e.ts:23`
- **F-008**: G-10d 的跨用户业务输入来自 Observer 新消息，但 Actor 的离底 anchor
  必须稳定并只出现 newer hint；这属于“远端事实传播 + 本地导航隔离”，不能只归为
  sender/receiver 消息显示。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g10d-dual-real.e2e.ts:10`
- **F-009**: G-15 dual-real 当前证明双方 topic dialog identity 收敛，但
  thread 数据只在 Actor 侧断言；Observer 的 thread 数据与成员角色视角
  尚未被该场景闭合。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g15-dual-real.e2e.ts:19`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g15-dual-real.e2e.ts:59`
- **F-010**: G-16 dual-real 已区分 leaver 与 remaining member 的不同生命周期，并验证
  close 后双方收敛 closed；这证明频道/成员 Gate 必须按角色定义终态，不能复制同一 VM。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g16-dual-real.e2e.ts:19`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g16-dual-real.e2e.ts:81`
- **F-011**: G-16c dual-real 当前只闭合 `displayName` 广播；群简介与公告虽属于同一
  设置族，但本场景没有对应双端断言。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g16c-dual-real.e2e.ts:12`,
  `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g16c-dual-real.e2e.ts:44`
- **F-012**: 远端业务库的只读真源为 PostgreSQL
  `10.200.9.168/32:5432/mattermost-imv2/public`；G01-G07/G14 的核心持久事实分布于
  `post/channel_member/channel_event/delivery_outbox`，不能从 WS 样例反推表列。 |
  **Evidence**: `specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json:5`
- **F-013**: 本机当前可审阅的 Helix tenant DB 为用户 444 与 999 两库，均位于
  `~/Library/Application Support/com.jinqidongli.cses.dev/helix_im`，并共享
  `message/channel/channel_member/channel_schedule/channel_event_*` 等 9 张表。 |
  **Evidence**: `specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json:15`
- **F-014**: `channel.last_post` 与 `channel_member.last_post` 不是拆列对象，而是保存
  camelCase post JSON 的 JSONB/JSON-text；其已观测键形包含 `id/temporaryId/channelId/
  userId/message/revoke/createAt/updateAt`。 |
  **Evidence**: `specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json:304`
- **F-015**: 本地 `message` 表已有独立 `quick_reply/read_bits/event_seq/send_status/revoke`
  列；这些字段不应再往 `props` 塞第二份别名。 |
  **Evidence**: `specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json:355`
- **F-016**: G04 的 `system_event` 是 Go 在撤回事务提交后构造并注入 WS 的传输派生；
  远端持久锚为 `post.revoke` 与 `channel_event.actor_id/event_seq/event_type`，本地持久锚为
  `message.revoke`。展示文案不在上述表中。 |
  **Evidence**:
  `/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_revoke.go:216`,
  `specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json:63`

## Unknowns

- **U-001**: 截图遗漏的 G-03 是否立即加入这批人工评审和后续补测分母。 |
  **Resolution**: 已确认不加入；本轮保持 `OUT_OF_SCOPE`，不得创建占位测试或 JSON。
- **U-002**: G-17 至 G-27 是否应在下一轮按相同双视角模板继续展开。 |
  **Resolution**: 已确认不加入；后续必须另行明确范围，不得计入本轮完成率。
- **U-003**: 定时消息真正到点投递、取消与触发竞态、owner 退出后的角色转移是否作为
  独立 Gate。 |
  **Resolution**: 产品人工 Gate；当前仅标记 `MISSING/INFERRED`，不冒充既有合同。
- **U-004**: G-16c 的群简介与公告是否要求与 `displayName` 相同的双端时序和
  capability 规则。 |
  **Resolution**: 用户确认字段级产品合同，当前只把源码未覆盖事实钉住。

## Decisions

- **D-001**: 本规格主表以截图 30 Gate 为人工审阅分母，同时单列当前 Matrix
  50/47/3 分母与差集，禁止混算完成率。
- **D-002**: 统一使用 Actor/Observer 作为角色上位词；只有消息投递时再细化为
  Sender/Receiver，恢复、分页、定位、已读和定时隐私不强套双向传播。
- **D-003**: 当前覆盖统一记录为 `SOURCE_PRESENT / NOT_RUN`；fixture、静态源码、
  Mock、single-real 和 dual-real 不互相替代。
- **D-004**: 当前场景没断言的 receiver/角色行为标记 `MISSING`；由相邻业务不变量
  推导的目标标记 `INFERRED`，等待用户确认后才能进入计划。
- **D-005**: 本轮在 Helix 仓内完成 30 Gate 的生产接码、独立 Rust 测试与 `.v1`
  产物生成；不修改或同步 `cses-client` WDIO 与 Mobile GTest。
- **D-006**: 以 `contracts/storage-schema-lock.v1.json` 冻结本轮只读表结构；每个 Gate
  的 inbound/outbound 样例必须引用该锁并列出逐列行结构，未映射字段显式进入派生清单。
- **D-007**: 本轮唯一实现分母为 spec 冻结的 30 Gate。G-03 和 G-17 至 G-27 只作为
  排除边界记录，不创建测试 target、generated 目录或空产物。
- **D-008**: 当前历史生产测试、canonical fixture 与 WDIO 场景均只证明入口存在；
  只有本轮按六级模型实际执行并登记的结果才能提升 Gate 证据等级。
