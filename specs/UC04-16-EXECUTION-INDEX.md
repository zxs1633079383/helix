# UC04-16 / G04-G16 执行索引

**日期**: 2026-07-22
**状态**: Draft，规格拆分完成，尚未声明任何 Gate Runtime Green
**目标**: Helix 产生 render-ready InteractionFrame，cses-client PC 机械消费并完成 Message-v4 UI；所有 mutation 以远端权威事实、本地 PersistOk 和匹配 causation 的 Frame 为业务完成。

## 1. 编号口径

宏 UC 与 Strict Matrix Gate 不是一一同号。权威映射中 `UC04=G03a 重发`，而本批从 `G04/G04a=UC05 撤回` 开始。为满足“按 G04-G16 顺序拆分”，本目录按 Gate 编号建 13 个独立 feature；最终总验收仍覆盖 UC04 的既有 G03a 回归，避免把“G04”误写成“UC04”。

| 顺序 | Feature | 宏 UC | 能力 |
|---|---|---|---|
| 004 | [G04 撤回](./004-g04-revoke/spec.md) | UC05 | 在线/离线撤回 |
| 005 | [G05 转发](./005-g05-forward/spec.md) | UC06 | 逐条/合并、多目标局部成功 |
| 006 | [G06 Reaction](./006-g06-reaction/spec.md) | UC07 | emoji 聚合与 viewer selected |
| 007 | [G07 加急](./007-g07-urgent/spec.md) | UC08 | 发起、确认、取消、双视角回执 |
| 008 | [G08 创建定时](./008-g08-schedule-create/spec.md) | UC09 | 定时创建、owner-only banner |
| 009 | [G09 取消定时](./009-g09-schedule-cancel/spec.md) | UC09 | 取消、编辑回填、到点发送收敛 |
| 010 | [G10 时间线窗口](./010-g10-timeline-window/spec.md) | UC10 | 首屏、上拉、向后补窗 |
| 011 | [G11 定位](./011-g11-locate-message/spec.md) | UC11 | 本地优先、远端补齐、reveal |
| 012 | [G12 回复树](./012-g12-reply-thread/spec.md) | UC12 | root、分页、分支、主区同步 |
| 013 | [G13 会话已读](./013-g13-channel-read/spec.md) | UC13 | viewport/focus 到 unread 原子归零 |
| 014 | [G14 回执/模板](./014-g14-receipt-template/spec.md) | UC13/UC14 | 单条已读、回执查询、模板 received 边界 |
| 015 | [G15 建群/话题](./015-g15-channel-topic/spec.md) | UC15 | G15a 建群整合与 G15 话题 |
| 016 | [G16 群治理](./016-g16-channel-governance/spec.md) | UC16 | close/leave/member/settings |
| Final | [Native E2E 收敛修复](./017-uc04-16-native-e2e/spec.md) | UC04-UC16 | 连续运行、日志/双库诊断、跨仓修复、重建重验直到 Full Green |

## 2. 每个 Gate 的固定交付

每个 feature 必须同时沉淀：`ActionBinding -> ActionInvocationEnvelope -> Tick::Command -> Effect -> authoritative HTTP/WS/sync -> PersistAtomic -> PersistOk -> SurfaceComposer -> InteractionFrame -> Angular SurfaceRuntime -> Message-v4 Component/UI`。HTTP 200、CommonRes `SUCCESS`、raw WS、日志、单独 DB 行或 action receipt 都只是中间证据。

每个 Gate 的 `spec.md` 固定业务合同；`plan.md` 固定领域边界、依赖方向、状态/复杂度和文件影响面；`tasks.md` 将 ActionBinding、Tick、Persist、VM/Frame、Angular/UI、focused tests 拆成可独立完成的小任务。

## 3. 编译与验收策略

- 单 Gate 开发阶段：写失败 fixture/contract/replay，再实现；只跑不触发完整 Native 重编的 focused 检查。中途编码禁跑 cargo。
- 完成一个 crate 内的一批 Gate 后：允许一次 crate 级 `cargo check`，不按 UC 重复运行。
- G04-G16 全部 `READY_FOR_FINAL_E2E` 后：进入 017，连续运行双账号 Native 产品链；失败后按日志、远端 PostgreSQL、本地 SQLite、Frame/DOM 和设计证据直接修复 Helix、Go server 或 cses-client。每个 repair batch 只增量 rebuild 一次，不按 UC 重编，循环到 final candidate Full Green。
- 运行失败时：读取 Go server 与 cses-client 当前任务时间窗内的结构化日志，按 correlation/trackId 定位后修复；日志仅用于诊断，修复后必须重跑失败 Gate 及受影响回归，不能用日志替代结果。

## 4. 一次性真实验收输入

- Actor A: user/cookie identity `444`
- Actor B: user/cookie identity `678`
- Company: `64118eebd2b665246b7880eb`
- 密码不写入仓库；从运行时安全环境注入。
- 视觉真源: `http://localhost:63347/message/pages/message/uc08-26/index-anchored.html`
- UI Handoff: `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`

## 5. 共同真源

- `crates/helix-im/docs/interaction-v2/完整时序和数据/`
- `docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`
- `crates/helix-im/src/interaction_frame.rs`
- `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/`

`pd/message` 只提供设计参考，不是业务事实源；Go server 是远端协议/持久事实源；Helix 是业务决策、持久屏障与 Frame 真源；Angular 只保留平台 overlay、draft、route、DOM 和可访问性。
