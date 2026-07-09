# Task 5 报告：Cross-Repo Contract Matrix

## 状态

- 结论：`DONE_WITH_CONCERNS`
- 工作区：`/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709`
- 分支：`codex/dialog-projection-sync-spec-20260709`
- 启动自检：`pwd` 与 `git rev-parse --abbrev-ref HEAD` 均符合要求

## 完成内容

- 新建 [docs/specs/2026-07-09-dialog-projection-closure-matrix.md](/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-dialog-projection-closure-matrix.md)，汇总 Go / Helix / mobile / cses-client 四仓的字段责任、验证命令与最终状态。
- 在计划文件 [docs/superpowers/plans/2026-07-09-dialog-projection-sync-closure.md](/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/superpowers/plans/2026-07-09-dialog-projection-sync-closure.md) 将 Task 5 三个步骤标记为已完成，并与 matrix 一并纳入 docs commit。
- 矩阵中将 `readLastSeq/msgCount` 明确记为 `PARTIAL` / `NEED` pending，并把 Go DB integration 维持为 `SKIP`，没有写成 live green。
- 根据 reviewer finding 修复 Go `-run` 正则命令在 Markdown 表格里被写成 `&#124;` 的问题：带 `|` 的完整命令已移出表格，改为下方 fenced code block，表格只保留 `G1/G2/G3` 编号与摘要，复制命令时可得到真实 `|`。

## 影响面分析

- codebase-memory-mcp：
  - `crates/helix-im/src/channel/gate.rs::emit_dialog_patch_for_post` 是 Helix 本地从 post/sync 事件通向 `im:channel:update-by-post` 的入口。
  - `crates/helix-im/src/acl/projection_control_effects.rs::emit_channel_update_by_post` 负责产出 render-ready DialogPatch 事件。
  - `crates/helix-im/src/acl/projection_control_effects.rs::emit_sync_too_long` 负责产出 `im:sync:too_long(mode=drop_and_reload)`。
- 基于上述调用链，matrix 的职责边界按“Go 提供事实 -> Helix 做业务投影 -> mobile/cses-client 只透传/渲染”归档，没有扩写到未在链路上的模块。

## 验证

- `git diff --check -- docs/specs/2026-07-09-dialog-projection-closure-matrix.md .superpowers/sdd/task-5-report.md` PASS
- Markdown table pipe scan PASS（表格内不再含 `&#124;`，完整 Go 命令在 fenced code block 中保留真实 `|`）
- `pwd` 包含 `/worktrees/dialog-projection-sync-spec-20260709` PASS
- `git rev-parse --abbrev-ref HEAD` = `codex/dialog-projection-sync-spec-20260709` PASS
- docs fix commit `c2c5edd7a39bb8278f77020ba283fe8d88f5114d` PASS

## Concerns

- `readLastSeq/msgCount` 仍依赖 [NEED_HELIX_CHANNEL_MEMBER_FIX.md](/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/NEED_HELIX_CHANNEL_MEMBER_FIX.md) 后续闭环，当前只能记 `PARTIAL`。
- Go 的 `CSES_IM_TEST_DSN` 未设置，因此 matrix 里只能标 `SKIP`，不能升级成 live DB green。
- mobile 的 `make gtest-js-spec` 仍有 pinned rev 环境 concern，但 focused contract gate 已 PASS。

## 提交

- `c2c5edd7a39bb8278f77020ba283fe8d88f5114d` `docs(spec): 修正文档表格中的 Go -run 管道符转义`
