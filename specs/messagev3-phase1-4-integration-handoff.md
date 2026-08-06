# MessageV3 Phase 1-4 Helix 全阶段领域-Gate 交接

更新时间：2026-08-04
状态：`PARTIAL_INTEGRATION_HANDOFF`

本文只记录 Helix producer/core/dispatch/WS 真实代码与证据，不把 Reference、Mock、单元测试或
HTTP 200 当作真实 Go/WS/Storage/Native/双 persona/canonical PASS。完整跨仓总表见 Client：
`specs/messagev3-phase1-4-integration-handoff.md`。

## 1. 仓库快照

| 项目 | 值 |
| --- | --- |
| 路径 | `/System/Volumes/Data/workspace/worktrees/messagev3-phase2-review/helix` |
| 分支 | `codex/0804-Phase2-merge3-4` |
| HEAD | `a6cb817b` |
| upstream | `NO_UPSTREAM` |
| 工作区 | `CLEAN` |
| Phase 3 merge | `f162cb0c` |
| Phase 4 merge | `a6cb817b` |

## 2. Foundation 复用

Phase 1-4 均复用一个 `helix-im` `ImModule`、一个 correlation map、一个 WS registry、一个
PortReply/Result dispatcher 和同一套 Post/Channel Projection。Phase 2 的 cursor、event_seq、gap、
TooLong、hydration 和 generation 隔离继续作为后续 Phase 的底座。

禁止新增第二 Runtime、第二 Command registry、第二 EventSink、第二 bus、第二 PostProjection 或
同义 channel/member command。

## 3. Phase/Gate producer 矩阵

| Phase | 领域 | Gate | Helix producer / dispatch 边界 | 状态 |
| --- | --- | --- | --- | --- |
| 1 | Channel | `G08a` | channel create command、HTTP effect、persist 后 `channelCreated` | `PARTIAL` |
| 1 | Post | `G01a/b/f/k/l,G02a/b/c,G03a/b,G05a-e,l` | send/retry/revoke/reaction/urgent/read 的既有 outbound 与 post handler | `PARTIAL` |
| 1 | Upload | `G01c/d/e/g,G02d,G03c` | Host media stage 与发送 effect；最终文件事实不进 Post intent | `PARTIAL` |
| 1 | Draft | `G02e` | Persist-only draft command/result，零领域事件 | `PARTIAL` |
| 1 | Schedule | `G04a-e` | schedule outbound、timer continuation、结构化查询 Result | `PARTIAL` |
| 1 | Announcement | `G07b` | Announcement command + PostProjection | `PARTIAL` |
| 1 | Gap | `G01h/i/j` | 仅 TASK_CARD 入站或 schema 合同；不得伪造创建链 | `DEFERRED_GAP` |
| 2 | Read | `G06a-e` | Go receipt authority 的 correlated read/result 与绝对 projection | `PARTIAL` |
| 2 | Timeline/Reply | `G11a-h,G12a-b` | bounded query、read-back、reply correlation、单 terminal | `PARTIAL` |
| 2 | Sync | `G13a-e` | runtime/hello/pong/WS gap 驱动 cursor/gap/TooLong/hydration | `PARTIAL` |
| 3 | Channel | `G08b-d` | topic/close/leave producer，`channelClosed` + terminal reason | `PARTIAL` |
| 3 | Settings | `G10a-g` | CAS、viewer-local 设置、能力 projection、channelUpdate | `PARTIAL` |
| 3 | ChannelMember | `G09a-e` | member/admin/nickname/increment/presence Result/event | `PARTIAL` |
| 4 | Pin/Bookmark/Announcement | `G05f-k,G07a/c/d` | 复用既有 outbound、post pin/read、announcement version events | `PARTIAL` |
| 4 | Search/Todo | `G14a-c,G15a-d` | 本轮不实现、不分析、不解除阻塞 | `BLOCKED/ROADMAP` |

## 4. 关键 Helix 语义

- `G08b` topic 保持 `channelCreated -> postUpdated`，只把 `rootPostId` 映射为 wire `root_id`。
- `G08c/G08d` 复用 `channelClosed`，终态原因严格为 `closed|left`；NOTICE 由 Go 唯一事务生成。
- `G09d` 的 increment hydration 保留 Phase 3 显式 `req_id` 与 `emit_channel_increment` correlation，
  结构化 Result 与 sender-only event 至多各结算一次。
- `G09e` 只返回 `online|offline|unknown`，不发领域事件。
- `G10a-g` 的版本 CAS、权限能力和 viewer-local 状态不由 Helix 猜测或复制另一 persona。
- Phase 4 Pin/Bookmark/Announcement 只沿用现有 registry、dispatch、PostProjection 和 receipt 链。

## 5. Evidence 状态

| 层级 | 状态 | 说明 |
| --- | --- | --- |
| Unit/Core | `PARTIAL/PASS_BY_SCOPE` | `cargo test -p helix-im --lib` 在合并检查点通过；不等于跨仓真实链 |
| Contract | `PARTIAL/PASS_BY_SCOPE` | Gate/Reference/wire 局部校验通过 |
| HTTP/WS/Storage | `NOT_VERIFIED` | 未形成 84 Gate 全量真实回放 |
| Native | `NOT_VERIFIED` | 未绑定桌面宿主运行报告 |
| Dual-user / multi-device | `NOT_RUN` | 未形成完整 persona/device 矩阵 |
| Canonical | `NOT_RUN` | 不得从 Helix 单测升级 |

## 6. 下一步与停止条件

真实 provider、Go receiver、认证、Storage DSN、runId/operationId、cleanup 或双 persona 缺失时，
保持 `BLOCKED/NOT_VERIFIED`。任何要求新 Runtime、Port、bus、registry、PostProjection 或
readBits 的任务立即停止并回交总表。

`PHASE1_4_HELIX_HANDOFF_PARTIAL`
