# Implementation Plan: InteractionFrame V2 26 Gate 三仓总控

**Branch**: `020-ifv2-three-repo-uc-control` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

## Summary

总控按 manifest 固定顺序调度 G01 基线与 25 个子 Feature。每次只激活一个 Gate；子 Feature 独立完成 Helix producer/Fixture、PC Mock/Real/report、Mobile GTest/QuickJS 与 Review。总控只读取 ledger/report/hash 形成状态，不实现任何 Gate 业务，不把低等级证据提升为 Green。

## Technical Context

**Language/Version**: Markdown/JSON/Node 20/Python 3；子 Feature 使用 Rust、TypeScript/Angular/Tauri 与 C++/QuickJS

**Primary Dependencies**: gate-spec-manifest、子 Feature review-ledger、cross-repo report、helix-loop

**Storage**: Spec Kit 文件与 hashed evidence；不直接写业务 SQLite/PG

**Testing**: JSON/schema/link/task-traceability、26 个 R2/R3、逐 Gate R4 汇总

**Target Platform**: Helix、cses-client、Mobile 三仓交付编排

**Project Type**: fail-closed program controller

**Performance Goals**: 按需只加载当前 Gate；状态汇总 O(26)，不扫描全量业务日志

**Constraints**: 严格串行 Gate；子证据不可互相替代；用户决定 legacy 删除；总控不进入生产业务路径

**Scale/Scope**: 26 Active、20 Gap、25 子 Feature、1 个 G01 baseline

## Constitution Check

| 原则 | 设计判定 |
|---|---|
| Layering | PASS：总控只读子证据，不承载业务 reducer |
| Determinism | PASS：顺序和状态来自机器 manifest/ledger |
| Frame V2 Truth | PASS：只接受 Helix canonical report/hash |
| Evidence | PASS：每 Gate required lanes 独立 |
| Minimal Migration | PASS：G03、G17-G27 保持 Gap |
| Loop Admission | PASS：每个 Feature 独立 R1-R4 |

**Post-design re-check**: PASS。总控 contracts 只定义编排状态与删除人类门，不修改三仓协议。

## Loop Detail Contract

### Inputs and Context

- **I-001**: [gate-spec-manifest.json](gate-spec-manifest.json) 的固定执行顺序和 Gap。
- **I-002**: [plan-task-index.json](plan-task-index.json) 的子 Plan/Task/ledger 路由。
- **I-003**: 每个子 Feature 的 review-ledger、verification 和 gate-report。
- **I-004**: G01 baseline `specs/019-g01-wdio-five-phases` 的当前候选复验结果。

### Action Chain

- **A-001**: 读取当前 Gate 与前序 ledger -> admission 判定 -> 激活一个子 Feature。
- **A-002**: 子 Gate R4/report/hash -> 独立 consistency review -> review-green 或 blocked。
- **A-003**: review-green -> 更新总控状态 -> 激活下一 Gate。
- **A-004**: 26/26 Green -> 生成 legacy 候选 -> 等待用户选择 -> 删除后重跑三仓。

### State Contract

- **S-001**: `Planned -> GateActive`：当前 Gate R1-R3 PASS，前序 review-green。
- **S-002**: `GateActive -> GateGreen`：该 Gate R4 与 cross-repo review PASS。
- **S-003**: `GateGreen -> NextGateActive`：严格按 manifest 前进。
- **S-004**: `26Green -> DeletionCandidateReady -> AwaitingUserSelection`。
- **S-005**: 用户批准后 `Deleting -> FullRegression -> Closed|Blocked`。

### Validators

- **V-001**: manifest/index/schema 校验证明 26 Active、25 子 Feature、20 Gap 唯一。
- **V-002**: 26 个 R2/R3 ledger 与 task contract 校验证明计划可实施。
- **V-003**: 每 Gate report schema/hash/required lane 检查证明 review-green 不越级。
- **V-004**: 删除候选反向引用与用户选择记录证明无越权删除。
- **V-005**: 删除后三仓全量回归证明最终候选仍 26/26 Green。

## Project Structure

```text
specs/020-ifv2-three-repo-uc-control/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
├── tasks.md
├── plan-task-index.json
└── plan-task-index.md

specs/021..045/
└── plan/research/data-model/contracts/quickstart/tasks
```

**Structure Decision**: 总控只保存索引、顺序、状态与删除人类门；所有测试和生产接线任务留在对应子 Feature。

## Delivery Phases

1. 规格和 R1（已完成）。
2. 总控与 25 子 Plan/R2。
3. 总控与 25 子 Task/R3。
4. 按 manifest 逐 Gate implementation/R4。
5. 26 Green 后 legacy 候选、用户选择、删除后全量回归。

## Complexity Tracking

无 Constitution 例外。26 个 Feature 的文档数量来自强隔离验收需求，不代表并行实现或复制业务逻辑。

## Child Plans

- `G-02`: [plan](../025-ifv2-g-02-document-message/plan.md) · [tasks](../025-ifv2-g-02-document-message/tasks.md)
- `G-03a`: [plan](../026-ifv2-g-03a-retry-failed-send/plan.md) · [tasks](../026-ifv2-g-03a-retry-failed-send/tasks.md)
- `G-04`: [plan](../027-ifv2-g-04-online-revoke/plan.md) · [tasks](../027-ifv2-g-04-online-revoke/tasks.md)
- `G-04a`: [plan](../028-ifv2-g-04a-offline-revoke/plan.md) · [tasks](../028-ifv2-g-04a-offline-revoke/tasks.md)
- `G-05`: [plan](../029-ifv2-g-05-forward-message/plan.md) · [tasks](../029-ifv2-g-05-forward-message/tasks.md)
- `G-06`: [plan](../030-ifv2-g-06-quick-reaction/plan.md) · [tasks](../030-ifv2-g-06-quick-reaction/tasks.md)
- `G-07`: [plan](../031-ifv2-g-07-urgent-message/plan.md) · [tasks](../031-ifv2-g-07-urgent-message/tasks.md)
- `G-08`: [plan](../032-ifv2-g-08-schedule-create/plan.md) · [tasks](../032-ifv2-g-08-schedule-create/tasks.md)
- `G-09`: [plan](../033-ifv2-g-09-schedule-cancel/plan.md) · [tasks](../033-ifv2-g-09-schedule-cancel/tasks.md)
- `G-10`: [plan](../034-ifv2-g-10-timeline-window/plan.md) · [tasks](../034-ifv2-g-10-timeline-window/tasks.md)
- `G-10a`: [plan](../035-ifv2-g-10a-load-older/plan.md) · [tasks](../035-ifv2-g-10a-load-older/tasks.md)
- `G-11`: [plan](../036-ifv2-g-11-locate-message/plan.md) · [tasks](../036-ifv2-g-11-locate-message/tasks.md)
- `G-12`: [plan](../037-ifv2-g-12-reply-thread/plan.md) · [tasks](../037-ifv2-g-12-reply-thread/tasks.md)
- `G-13`: [plan](../038-ifv2-g-13-channel-read/plan.md) · [tasks](../038-ifv2-g-13-channel-read/tasks.md)
- `G-14`: [plan](../039-ifv2-g-14-post-read/plan.md) · [tasks](../039-ifv2-g-14-post-read/tasks.md)
- `G-14a`: [plan](../040-ifv2-g-14a-template-received/plan.md) · [tasks](../040-ifv2-g-14a-template-received/tasks.md)
- `G-15a`: [plan](../041-ifv2-g-15a-create-channel/plan.md) · [tasks](../041-ifv2-g-15a-create-channel/tasks.md)
- `G-15`: [plan](../042-ifv2-g-15-create-topic/plan.md) · [tasks](../042-ifv2-g-15-create-topic/tasks.md)
- `G-16`: [plan](../043-ifv2-g-16-channel-close-leave/plan.md) · [tasks](../043-ifv2-g-16-channel-close-leave/tasks.md)
- `G-16b`: [plan](../044-ifv2-g-16b-member-leave-broadcast/plan.md) · [tasks](../044-ifv2-g-16b-member-leave-broadcast/tasks.md)
- `G-16c`: [plan](../045-ifv2-g-16c-channel-settings/plan.md) · [tasks](../045-ifv2-g-16c-channel-settings/tasks.md)
- `G-S1`: [plan](../021-ifv2-g-s1-cold-start-sync/plan.md) · [tasks](../021-ifv2-g-s1-cold-start-sync/tasks.md)
- `G-S2`: [plan](../022-ifv2-g-s2-offline-sync-notify/plan.md) · [tasks](../022-ifv2-g-s2-offline-sync-notify/tasks.md)
- `G-S3`: [plan](../023-ifv2-g-s3-pong-gap-recovery/plan.md) · [tasks](../023-ifv2-g-s3-pong-gap-recovery/tasks.md)
- `G-S4`: [plan](../024-ifv2-g-s4-unknown-channel-hydration/plan.md) · [tasks](../024-ifv2-g-s4-unknown-channel-hydration/tasks.md)
