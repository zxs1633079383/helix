# Helix 文档地图

> `docs/` 的单一入口。本文只维护 canonical 路由、证据层级和熵管理规则，不复制任务
> 流水或 Green 数字。
>
> 最近文档日检：2026-10-08，源码基线 `000d83a6`。全库建立 Markdown 清单并扫描相对链接；
> 本轮语义对账覆盖公共包版本、书签计数、成员稀疏更新、Module trait、CI 入口及索引。
> 历史转发附件无法复核，旧 Strict Matrix 生成文档 checker 失败；不代表全库 API 或运行态通过。
> 范围、证据与未验证项见[本轮交接](交接/Markdown日检-20261008/handoff.md)。

---

## 1. 冲突裁决顺序

| 层级 | 真源 | 负责回答 |
|---|---|---|
| L0 | 当前代码、Cargo/config、目录与运行 artifact | 现在实际执行什么 |
| L1 | Spec 047 migration manifest + machine evidence | 当前 MessageV3 Gate 的 producer、状态与证据等级 |
| L2 | [`GOAL.md`](./GOAL.md) | 为什么做、不可违反什么、TGCR 如何计算 |
| L3 | [`harness/decisions.md`](./harness/decisions.md) | 已拍板的 ADR 与边界 |
| L4 | Spec/Plan/Tasks | 某次 Feature 的范围、顺序、完成合同 |
| L5 | [`ROADMAP.md`](./ROADMAP.md) | 当前状态、缺失证据和下一步 |
| L6 | [`ROADMAP-full.md`](./ROADMAP-full.md) | 历史战略全景，不替代当前活账 |
| 历史 | Frame/Surface 设计、旧 Matrix、旧报告与旧 pin | 解释来源，不证明当前状态 |

规则：

- 实现事实冲突时，代码优先；目标冲突时，GOAL/ADR 优先。
- deterministic、contract、integration、runtime、dual-user 与 canonical consumer 是不同
  证据等级，禁止互相替代。
- 旧 evidence pack 只有在 revision、manifest hash 和证据窗口仍匹配时才能参与当前裁决。

---

## 2. 必读链

### 每次会话

1. 根 [`AGENTS.md`](../AGENTS.md)
2. [`GOAL.md`](./GOAL.md)
3. 根 [`SESSION.md`](../SESSION.md)
4. [`ROADMAP.md`](./ROADMAP.md)
5. 与任务对应的 crate-local `AGENTS.md`

### 做 MessageV3 进度或发布判断

1. [`gate-migration.v1.json`](../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json)
2. [`verification.md`](../specs/047-im-event-gate-inventory/verification.md)
3. [`review-ledger.json`](../specs/047-im-event-gate-inventory/review-ledger.json)
4. 当前代码、测试 target 与本轮 machine evidence
5. 下游当前消费 pin 与各自运行报告（需要跨仓判断时才读取）

旧 `strict-matrix.v1.json`、`command-api.v1.json` 与 `docs/Strict Matrix/` 已是
`RETIRED_HISTORICAL`，不能再作为当前 Gate 分母。

---

## 3. 当前架构入口

| 主题 | Canonical 文档 | 说明 |
|---|---|---|
| 北极星与 TGCR | [`GOAL.md`](./GOAL.md) | 运行时无关、三端同核、唯一北极星指标 |
| 当前活账 | [`ROADMAP.md`](./ROADMAP.md) | Spec 047 完成状态与仍缺的消费/运行证据 |
| ADR | [`harness/decisions.md`](./harness/decisions.md) | ADR-001..018；ADR-017 是当前 MessageV3 合同 |
| IM 当前入口 | [`crates/helix-im/docs/README.md`](../crates/helix-im/docs/README.md) | 当前运行时文档与历史 Frame 文档的边界 |
| MessageV3 实施真源 | [`specs/047-im-event-gate-inventory/`](../specs/047-im-event-gate-inventory/) | 30 Gate manifest、schema、任务与验证 |
| 可执行护栏 | [`harness/README.md`](./harness/README.md) | HX-C001..HX-C018 |

三层代码边界：

```text
helix-im（当前业务模块）
        ↓ only core ports
helix-core
        ↑ adapter implements ports
helix-driver-host / native / ffi / web
        ↑ protocol conversion and platform I/O only
PC / Mobile / Web shell
```

---

## 4. MessageV3 Gate Event 入口

| 文档/目录 | 角色 | 能否证明当前 Green |
|---|---|---|
| [`spec.md`](../specs/047-im-event-gate-inventory/spec.md) | 精确 30 Gate 与 MessageV3 目标合同 | 不能，属于需求合同 |
| [`gate-migration.v1.json`](../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json) | Gate、domain、test target、production owner 与迁移状态机器真源 | 只能证明登记状态 |
| [`message-v3-event.schema.json`](../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json) | `{event,data}` 出站 schema | 只能证明结构合同 |
| [`tests/message_v3/`](../crates/helix-im/tests/message_v3/) | 当前 producer 回放与各 Gate 独立 target | 可证明 deterministic/contract 面 |
| [`verification.md`](../specs/047-im-event-gate-inventory/verification.md) | R4 分层结果、物理删除与 required runs | 只证明其固定运行窗口 |

当前生产协议不包含 `InteractionFrame`、`Surface`、`SurfaceMutation`、`ActionBinding`、
`UiDirective` 或 FrameApplied。`interaction-v2/`、旧 Strict Matrix、Spec 018-046 与
`docs/interaction-frame-v2-*` 仅保留为显式历史 allowlist。

`.v1` 是 Spec 047 被 Git 忽略的本地 evidence/schema 版本目录，不是运行时 V1 协议。

---

## 5. 三端事实入口

### Helix

- core：`crates/helix-core/src/`，只拥有 Tick、Effect、ports、Module host 与执行壳。
- IM：`crates/helix-im/src/`，拥有 MessageV3 事件、持久屏障、查询、同步与恢复。
- host：`crates/helix-driver-host/`，提供共享 SQLite、HTTP、WS、timer、metrics 与执行驱动。
- native/FFI/Web：`crates/helix-driver-{native,ffi,web}/`，只兑现平台边界并原样交付事件 bytes。
- 当前四个 crates.io 公共包为 `cses-helix-core`、`helix-im`、`helix-driver-host`、
  `helix-driver-native`；本轮 Cargo manifest 版本均为 `0.1.48`，registry 状态未重新查询。

### PC / Mobile

- PC 与 Mobile 的当前 source pin、产品 UI 和真实运行证据属于各自仓库；跨仓判断必须读取
  它们当前 checkout，不从本仓历史文档复制旧 SHA。
- Spec 047 的 Helix 版本记录二者为 `NOT_SYNCED_BY_SCOPE`；不能把当前 Helix producer
  PASS 写成 PC/Mobile 已 byte-exact 消费。

### Web

- Rust 平台 driver：`crates/helix-driver-web/`。
- browser adapter/smoke：`crates/helix-driver-web/browser/`。
- Rust 平台壳、Browser Fetch、page-session store、WS bridge、正式 gate 与真浏览器→真 Go
  的鉴权、send/echo、断连恢复链已落地。
- crate 不包含产品 renderer；`Effect::UploadFile` 在 Web 仍返回 unsupported。因此 Web
  平台链落地不等于全部 Gate UI 或媒体能力完成。

---

## 6. 证据语言

| 状态 | 含义 |
|---|---|
| `deterministic-pass` | 同一 Tick/Effect/PersistOk/Emit 剧本稳定 |
| `contract-pass` | schema、字段、hash 与 Gate 集合通过 |
| `integration-pass` | crate/driver 接线通过 |
| `runtime-pass` | 指定宿主在固定窗口实际执行通过 |
| `dual-user-pass` | actor/observer 同窗传播通过 |
| `canonical-consumer-pass` | 下游对同一 producer bytes 完成镜像与消费 |
| `NOT_RUN` | 该证据面未执行 |
| `NOT_SYNCED_BY_SCOPE` | 当前版本明确不负责同步该消费者 |

只有 GOAL §5 所列条件在同一 candidate/manifest/window 内同时满足，Gate 才进入 TGCR 分子。

---

## 7. 其他领域入口

| 主题 | Canonical 文档 |
|---|---|
| 当前 IM 文档边界 | [`crates/helix-im/docs/README.md`](../crates/helix-im/docs/README.md) |
| 在线/离线消息发送 | [`在线与离线发送消息全流程.md`](../crates/helix-im/docs/消息发送全流程/在线与离线发送消息全流程.md) |
| 频道同步分页 | [`频道同步分页协议.md`](../crates/helix-im/docs/频道同步分页协议.md) |
| 分类接龙接入与历史验收边界 | [`category-chain-client-contract.md`](./category-chain-client-contract.md) |
| UC/HTTP/WS/storage 历史代码分析 | [`前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](../crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md) |
| 历史真机 curl wire 参考 | [`真机curl真源.md`](../crates/helix-im/docs/csesapi-migration/真机curl真源.md) |
| FFI 合同 | [`crates/helix-driver-ffi/docs/`](../crates/helix-driver-ffi/docs/) |
| Metrics 与用户/群聊诊断入口 | [`Metrics/README.md`](./Metrics/README.md) |
| Runtime Metrics | [`Metrics/Prometheus指标说明.md`](./Metrics/Prometheus指标说明.md) |
| MessageV3 观测环境迁移 | [`Metrics/MessageV3全链路观测环境迁移手册.md`](./Metrics/MessageV3全链路观测环境迁移手册.md) |
| Harness | [`harness/README.md`](./harness/README.md) |
| 提交规范 | [`COMMIT_CONVENTION.md`](./COMMIT_CONVENTION.md) |

---

## 8. 熵管理

1. **一事实一真源**：目标只进 GOAL，决策只进 ADR，Gate 状态进 manifest/evidence，当前摘要进 ROADMAP。
2. **不复制动态数字**：除带日期、revision、manifest hash 的快照外，不在叙事文档复制 Green 数量。
3. **过期即降级**：旧 pin、旧 Matrix、旧协议和旧运行窗口必须标记历史，不继续作为当前入口。
4. **目标与现状并列**：Spec TARGET、代码 CURRENT 与未兑现 GAP 必须明确分开。
5. **删除优于堆叠**：被 ADR 覆盖的路线从 active 入口删除；有追溯价值时只保留历史链接。
6. **新文档准入**：新建 Markdown 前先证明无法进入现有 canonical 文件。
7. **入口同步**：新增、移动或删除 canonical 文档时同步本文、局部 README 与链接检查。
8. **断链与漂移检查**：提交前检查 Markdown link、ADR/HX-C 数量、Gate manifest、退役术语与 `git diff --check`。
9. **证据不过夜升级**：实现提交可以先于真实证据，但 ROADMAP 必须保留精确分面。
10. **TGCR 单一出口**：周报、Roadmap 或发布稿只引用 GOAL §5 的机器派生口径。
