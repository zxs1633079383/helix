# Helix Harness — 运行时无关核心层的反模式自维护约束库

> **scope**：`crates/helix-core` + `crates/helix-im` + `crates/helix-driver-*`
> **母本（方法论）**：[`/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md`](/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md)
> **项目指令**：[`../../CLAUDE.md`](../../CLAUDE.md)（§4 索引指针 + 双端 checklist）
> **详细规约**：[`../../.claude/rules/runtime-agnostic-core.md`](../../.claude/rules/runtime-agnostic-core.md)
> **触发操作手册**：[`OPERATING.md`](./OPERATING.md)（什么事 → 调什么；自动 vs 人工触发矩阵）

本目录是 Helix「心脏血脉生命线」的可执行护栏：每条卡片把一个**双端边界条件 / 架构不变量**编译成 grep gate + CI gate + 正反代码对照，让坑写不出来、不重复踩。历史 wasm 护栏按 ADR-006 归档保留，不作为当前产品验收。

## 1. 三个即时同步铁律（最高优先级，来自母本 §0）

```
① 踩坑 → 即时写 /workspace/java/logs/{date}.json
② 同根因 ≥3 / 用户明确 / Spec 拍板 → 即时新建 HX-C{NNN}-*.md（同会话内，不等"以后"）
③ harness/ 增删 → 即时同步 CLAUDE.md §4 索引表 + 本 README §3 + 一次 commit
```
断链后果：①断→忘事；②断→重复踩坑；③断→Claude 读不到等于没写。

## 2. 设计原则（5 条）

1. **触发驱动**：同根因 ≥ 3 次 / 用户拍板 / Spec 才入库。初始卡片例外（来自 2026-06-08 多专家 review 的真实 blocker/major，有证据）。
2. **可执行 > 可读**：每条必须有真实代码片段 + grep/cargo 命令 + checklist。
3. **自维护**：复现一次 → 同 commit 内 `recurrence_count +1` + `log.md` 追行 + 卡 §5 追行。
4. **有生命周期**：drafting → active → merged（30 天零命中 + inline rules/）/ deprecated（场景消失）。
5. **scope 隔离**：trigger_paths 收窄到本仓 crates/；要更广 → 走 graduates_to 晋升到 `~/.claude/rules/`。

## 3. 在册约束索引表（与 CLAUDE.md §4 镜像）

| ID | Status | Severity | Title | 检测手段 | Recurrence | Last |
|----|--------|----------|-------|---------|------------|------|
| [HX-C001](./HX-C001-core-zero-runtime-dep.md) | active | blocker | helix-core 零运行时/零平台依赖 | `cargo tree`+grep gate | 1 | 2026-06-08 |
| [HX-C002](./HX-C002-async-send-boundary-no-lifetime.md) | active | blocker | async port Send 边界 + 状态禁 `&'a` | grep + 双端 check | 1 | 2026-06-08 |
| [HX-C003](./HX-C003-wasm-cfg-must-ci-typecheck.md) | archived | blocker | wasm-only 代码必须进 CI type-check + compile_error 守护 | CI matrix | 1 | 2026-06-15 |
| [HX-C004](./HX-C004-features-additive.md) | active | major | features 必须 additive，禁互斥 | feature matrix build | 1 | 2026-06-08 |
| [HX-C005](./HX-C005-hotpath-zero-alloc.md) | active | major | 热路径零分配 O(1)（`&'static str` 不 String）| grep | 1 | 2026-06-08 |
| [HX-C006](./HX-C006-ffi-boundary-discipline.md) | active | blocker | FFI 边界铁律（no unwind / 单 owner / spawn_blocking）| grep + review | 1 | 2026-06-08 |
| [HX-C007](./HX-C007-effect-batch-across-boundary.md) | active | major | Effect 跨 FFI 边界批处理，不逐条 | review | 1 | 2026-06-08 |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l` == CLAUDE.md §4 表行数（都应=7）。不等 = 漂移，立即修。

## 4. 目录结构

```
docs/harness/
├── README.md            ← 本文（Layer B 索引）
├── TEMPLATE.md          ← 7 节卡片模板
├── decisions.md         ← 关键架构拍板记录（ADR-lite）
├── log.md               ← 跨卡复现日志
└── HX-C001 .. HX-C007   ← 7 条 active 约束
```

## 5. 生命周期

```
drafting → active → merged（30 天零复现 + grep gate 稳定 + inline 进 ~/.claude/rules/）
                  → deprecated（场景消失，如某 target 弃用）
```
晋升 merged checklist：§4 grep 命令存在 / CI gate `wc -l == 0` 放行 / §5 ≥1 条且最后一条 ≥30 天前 / 已 inline rules/ / frontmatter status=merged。

## 6. 何时深读母本

autonomous 任务开始前（协议 P1-P5）/ 第一次建卡（母本 §4+§5）/ 升级 merged·deprecated（§3.3）/ 周复盘自检（§8 闭环验证）。
