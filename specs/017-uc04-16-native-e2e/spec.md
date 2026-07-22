# Feature Specification: UC04-16 Native E2E 收敛修复

**Feature**: `017-uc04-16-native-e2e` | **Created**: 2026-07-22 | **Status**: Draft

**Loop Profile**: `helix-loop/v1`

## Purpose

本 feature 是 G04-G16 的最终执行与收敛 owner，不是只读验收报告。它在各 Gate focused implementation 完成后，一次启动双账号 Native 产品链，连续执行 UC04-UC16；发现失败后读取当前源码、Go server/cses-client 日志、远端 PostgreSQL、本地 SQLite、InteractionFrame 和 DOM/截图证据，直接修改 Helix、cses-im-server 或 cses-client，重新构建并验证，持续循环到全量 required Gate 为 Green。

017 不把十三个 Gate 合并成一个开发任务。每个失败仍回写到对应 G feature 的 ActionBinding、Tick、Persist、Surface/VM、Frame 或 Angular/UI task；017 只拥有跨 Gate 的运行、诊断、修复编排、回归和最终封板。

## Authority

017 被授权：

- 读取和修改 `/System/Volumes/Data/workspace/rust/helix`、`/System/Volumes/Data/workspace/golang/cses-im-server`、`/System/Volumes/Data/workspace/angular/cses-client` 中与 G04-G16 根因直接相关的代码、fixture、测试和文档。
- 读取当前验收时间窗内的 Go server、Helix host/native、Tauri、Angular、PostgreSQL 和 SQLite 日志/数据。
- 启动、停止和重启本次 E2E 所需的本地进程，并在修复批次后执行增量 rebuild。
- 按 Index-Dual 设计页修复 Message-v4 样式、状态、ARIA、稳定 data-* 与交互。

不得破坏脏工作树、提交或暴露凭据，也不得为了绿灯弱化断言、跳过 required UC、伪造 DB/Frame/DOM 证据或绕回 legacy reducer。

## Entry Gate

- 004-016 的 focused Rust/Angular 合同任务已完成，或缺项已被明确映射为 017 的首轮修复输入。
- UC04/G03a 重发回归已登记。
- Helix、Go server、cses-client 当前 revision、fixture manifest 和初始 consumer pin 已记录。
- 双账号 `444`、`678` 与 company `64118eebd2b665246b7880eb` 可用；密码仅从运行时安全环境注入。

## User Scenario

444 与 678 以两个独立 Native profile 连续完成 UC04-UC16。每个动作同时观察 actor/observer 的远端权威事实、本地 PersistAtomic、matching InteractionFrame、PC DOM 和实际交互。UI 视觉以 Index-Dual 为参考，但业务成功只由真实 Go authority + local PersistOk + matching Frame + DOM 建立。

## Mandatory Repair Loop

1. **Run**: 在同一 candidate 上连续运行 UC04-UC16，不按 UC 重编 Native。
2. **Classify**: 用 trackId/corr_key/causationId 把失败定位到 action/tick、Go HTTP/WS、normalize、Persist、projector/frame、SurfaceRuntime/apply、DOM/interaction 或 visual。
3. **Repair**: 在真实 owner 仓实施最小根因修复，并补可证伪回归测试；不能只改 E2E expectation。
4. **Rebuild**: 每个修复批次只构建受影响 target；Native 变更合并成一次增量 rebuild，不为单 UC 重编。
5. **Verify**: 先重跑失败 Gate和依赖回归；触及共享 contract/Persist/Frame/SurfaceRuntime 时重跑 UC04-UC16 全批。
6. **Repeat**: 仍有 required fail/skip、DB/Frame/DOM 不一致或 Index-Dual 关键交互差异时回到第 2 步；同一 task/validator/失败签名最多自动修复两次，第三次转入 NeedsHuman，不得无限自循环。

不得在“已发现问题”“已修一部分”“视觉 partial”或某一证据面 Green 时结束。

## Loop Contract

### Trigger

004-016 focused readiness 已完成，或缺口已明确登记为 017 首轮输入；双账号、三仓 revision、consumer pin、Go/PG/SQLite 与设计基准均已完成 preflight。

### Goal

在同一 final Native candidate 上完成 UC04-UC16 全批，发现问题时按第一处 authority divergence 归属 owner，先补红灯再修复，并让 ActionBinding、Tick、Persist、Surface/VM、Frame、Angular/DOM 与 Index-Dual 证据收敛。

### Completion

最终固定 pin 的 required fail/skip 为 0，actor/observer 的 Go/PG、SQLite、Frame、DOM 和 visual 同窗一致；所有命令、耗时、重试与修复哈希进入 evidence ledger。

### Boundaries

017 只拥有跨 Gate 运行与根因修复编排，不把十三个 Gate 合并实现；不弱化断言、不跳过 required UC、不覆盖三仓既有脏改动、不暴露凭据、不以 HTTP 200 或截图单面证明 Green。

### Human Gate

同一 task/validator/失败签名自动修复两次仍未通过、需要改变既定 Go 领域语义、执行不可逆迁移、扩大权限或遇到真实外部不可恢复阻塞时，写入 NeedsHuman 并停止该失败分支；其他独立 Gate 可继续收集非冲突证据。

## Requirements

- **FR-001**: 017 MUST 接管 004-016 遗留缺口，并允许在真实 owner 仓修改与 G04-G16 根因相关的代码、fixture、测试和文档。
- **FR-002**: 每个 candidate MUST 复用双账号进程连续执行 UC04-UC16，不得按 UC 构建或重启 Native。
- **FR-003**: 每个失败 MUST 以当前 runId 的日志、远端 PG、本地 SQLite、Frame 和 DOM 证据定位到明确 layer/owner/root cause。
- **FR-004**: 根因修复 MUST 修改生产路径并新增可证伪回归；不得只改 E2E expectation、skip 或弱化断言。
- **FR-005**: 构建 MUST 按 repair batch 聚合；每批最多一次受影响 target rebuild，最终 candidate 再进行一次完整 Native rebuild。
- **FR-006**: 修复后 MUST 先验证失败 Gate/依赖 Gate；共享 contract/Persist/Frame/SurfaceRuntime 变化 MUST 重跑全批。
- **FR-007**: 最终每个 Gate MUST 具有 actor/observer、Go/PG、SQLite、Frame、DOM 和 visual 同窗证据。
- **FR-008**: Index-Dual 关键样式、状态和交互 MUST 被实现并在桌面/移动双尺寸重验，不得以报告 partial 代替修复。
- **FR-009**: 执行 MUST 保留三仓既有脏改动并保护凭据，日志和证据必须脱敏。
- **FR-010**: 只要 required fail/skip、六面证据冲突或关键 visual partial 非零，017 MUST 继续修复循环，不得 Seal。

## Completion

只有最终修复后的同一 candidate 在同一证据窗口满足以下条件才完成：

1. Helix final checks、contract/replay tests 与 cses-client consumer pin 一致。
2. 最终 candidate 完成一次 Native rebuild 后，UC04-UC16 全批 `required fail=0, required skip=0`。
3. Actor/observer 的 Go HTTP、WS/sync 与远端 PostgreSQL 事实一致。
4. 两 profile 的本地 SQLite entity/cursor/revision 与 PersistAtomic write-set 一致。
5. ActionBinding、Tick、Frame causation、Surface/完整 VM row、apply/ack 无 gap、partial apply 或 legacy reducer。
6. Message-v4 DOM/ARIA/data-*、成功/失败/恢复交互正确。
7. Index-Dual 与真实页面在 `1440x900`、`390x844` 的关键样式、状态和交互均为 `aligned`；非关键像素差异有明确 waiver。
8. 所有修复、重跑、timeout、retry、blocked 与 wall-clock 时间进入最终 Strict Matrix/evidence ledger。

## Failure and Stop Policy

- 普通代码、合同、数据、环境接线或 UI 问题不构成停止理由，必须修复后继续。
- 外部系统不可用时先完成可在本地推进的诊断、修复和 fixture 回归；恢复后继续真实链。
- 无法由当前三仓代码、当前授权或可恢复环境解决的外部阻塞，或同一 task/validator/失败签名两次自动修复仍失败时，必须记录完整证据并进入 NeedsHuman；不得进行第三次自动修复。
- 日志、HTTP 200、fixture、单库查询或截图都不能单独证明 Green。

## Success Criteria

- **SC-001**: 不按 UC 重复 Native build；每个修复批次最多一次受影响 target build，最终 candidate 一次全量 rebuild。
- **SC-002**: UC04-UC16 required 场景全部具有 actor/observer、remote PG、local SQLite、Frame、DOM、visual 六面关联证据。
- **SC-003**: 每个运行失败都有 layer classification、owner、修复 diff、回归测试和 rerun 结果，未修失败数为 0。
- **SC-004**: reachable legacy/raw business reducer 为 0，成功态只由 matching render-ready Frame 建立。
- **SC-005**: 最终 Strict Matrix 在固定 pin 上为 Full Green，所有证据可追溯到哈希与时间窗。
