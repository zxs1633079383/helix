# MessageV3 Phase 4 三域 Helix 交接

更新时间：2026-08-03

## AGENT_HANDOFF/v1

```text
AGENT_HANDOFF/v1
state: partial
phase: 4
repository: helix
phase_denominator: 16
active_scope: pin, bookmark, announcement
blocked_domains: search, todo
phase2_status: PARTIAL
phase2_canonical: 0/20
merge_commit: 61645f14
next-owner: messagev3-phase-supervisor -> messagev3-implement-runner
stop: 不得推进 Search/Todo；不得重建 Runtime、Port、单总线、Command Result、PostProjection 或 readBits
```

## Phase 2 基础合并

- 来源分支：`origin/codex/messagev3-phase2-review-20260731`
- 来源 HEAD：`9054f98fb0bd5140ce377d8c4d0f21aecbba4913`
- Phase 4 合并前 HEAD：`ddd4e5e6aae1`
- 实际 merge commit：`61645f14`
- Helix 无文本冲突。
- 实际合入范围：`helix-im` 的 `channel_sync`、read/timeline/sync query、absolute projection、
  event/WS handler、hydration 和受影响 `helix-im` tests；未修改 Phase 4 spec/plan/tasks/manifest/
  contract/TARGET_REFERENCE，未触碰 Search/Todo。

## 复用矩阵

| 已有基础 | Phase 4 复用 | 未覆盖 |
|---|---|---|
| Phase 1 Runtime、单总线、Command Result | Pin/Bookmark/Announcement 继续使用既有入口 | 三域真实 command/event parity |
| Phase 2 sync/cursor/hydration、timeline window、分页 | Pin 查询和 Announcement 列表读取复用 | 三域真实 HTTP/WS/Storage、Native、双用户 |
| PostProjection、read receipt/readBits | Pin NOTICE 和公告确认复用既有权威 | 不新增第二套 PostProjection 或 read authority |

Phase 2 日志和测试只证明 Phase 2 观察结果，不证明 Phase 4 runtime、dual-user、Native 或
canonical。完整 Phase 4 分母仍为 `16/16`；当前只授权 Pin、Bookmark、Announcement 的 9 个
Gate，Search/Todo 的 7 个 Gate 继续 `BLOCKED`。

## 下一 owner 与停止条件

下一 owner 按 active Gate 白名单执行 Pin、Bookmark、Announcement。若实现要求新建第二套基础设施，
或依赖 Search/Todo 的任何命令、事件、过滤和路由，立即停止并报告精确阻塞点。
