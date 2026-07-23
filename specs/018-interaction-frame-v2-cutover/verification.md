# Verification: InteractionFrame V2 双仓硬切

## Phase 1 Source Lock

**Captured**: `2026-07-23T03:52:13-07:00`

| Repository | Source branch | Source HEAD | Source worktree state | Isolated branch | Isolated path |
|---|---|---|---|---|---|
| Helix | `main` | `3b430c6cb1cd4af615d487dde2c9788b7cfda257` | dirty: `SESSION.md`、WS parser/tests、G01 V2 流程图、loop ledger | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/rust/helix/worktrees/018-interaction-frame-v2-cutover` |
| cses-client | `feat/helix-reactor-im` | `8d71085453c62b966d82b6c947f68a20cc638e3f` | dirty: message-v4 runtime view/content/SCSS 六项 | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/angular/cses-client/worktrees/018-interaction-frame-v2-cutover` |
| cses-im-server | `main` | `6ddb5245364128732fea1c1ea48977001394a76f` | untracked `=`；本 Feature 默认只读 | 不创建 | `/System/Volumes/Data/workspace/golang/cses-im-server` |

两个隔离 worktree 创建后均为 clean；源工作树 branch/HEAD 前后未变化。所有实现、测试和提交只在隔离分支发生，除非最终 pin/验收步骤明确要求回写。

## Frozen Inputs

| Input | Value |
|---|---|
| cses-client `helix-core` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-im` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-host` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-native` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| source `src-tauri/Cargo.lock` SHA-256 | `a789269b5fc2edd1a09c6bb3114b60a2658b7e76fb097017a4c7fea6a43ed6f9` |
| `strict-matrix.v1.json` SHA-256 | `d3461beb760255f6ae733e3b5f7ceccd32dca22faf8adf0c76c29e0d6786b345` |
| InteractionFrame fixture manifest SHA-256 | `71c281e5879ecf2d61f71363777f7b2b7915170403d33a0faff8964fcf81b1da` |

`Cargo.lock` 是 cses-client 源工作树的本地 ignored artifact，隔离 worktree 初始不存在；首次 Native 构建生成的新 lock 必须作为候选证据重新 hash，不能把上述 source hash 当作最终 pin。

## Phase Status

| Task | Status | Evidence |
|---|---|---|
| T001 | PASS | 三仓 branch/HEAD/dirty、四包 pin、lock/matrix/fixture hash 已记录 |
| T002 | PASS | [discovery.md](discovery.md) 已按当前锁定源码更新 V1/合法独立 v1 inventory |
| T003 | PASS | `python3 scripts/verify-interaction-frame-v2-index.py` 返回 19/19/19、25 Gate、T021-T077 完整 |
| T004 | PASS | [quickstart.md](quickstart.md) 固定当前可执行 focused、样式构建、Native、双库只读入口与 artifact root |
| T005 | PASS | Helix/cses-client 隔离 worktree 已创建且源 ref 前后不变 |

## Required Runs

后续 V-001 至 V-008 的原始输出必须写入 `evidence/`，并通过 `loop_engine.py record-evidence` 登记 hash。失败输出不得被摘要文本覆盖。
