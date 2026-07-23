# Helix Harness — 运行时无关核心层的反模式自维护约束库

> **scope**：`crates/helix-core` + `crates/helix-im` + `crates/helix-driver-*`
> **母本（方法论）**：[`/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md`](/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md)
> **项目指令**：[`../../AGENTS.md`](../../AGENTS.md)（§4 索引指针 + 多端 checklist）
> **触发操作手册**：[`OPERATING.md`](./OPERATING.md)（什么事 → 调什么；自动 vs 人工触发矩阵）

本目录是 Helix「心脏血脉生命线」的可执行护栏：每条卡片把一个**多端边界条件 / 架构不变量**编译成 grep gate + CI gate + 正反代码对照，让坑写不出来、不重复踩。正式 `helix-driver-web` 已落地，旧 W0→W4 路线继续失效；HX-C003 已重写并激活为 native test + wasm final build 双目标守护，不再指向历史骨架。

## 1. 三个即时同步铁律（最高优先级，来自母本 §0）

```
① 踩坑 → 即时写 /workspace/java/logs/{date}.json
② 同根因 ≥3 / 用户明确 / Spec 拍板 → 即时新建 HX-C{NNN}-*.md（同会话内，不等"以后"）
③ harness/ 增删 → 即时同步 AGENTS.md §4 索引表 + 本 README §3 + 一次 commit
```
断链后果：①断→忘事；②断→重复踩坑；③断→agent 读不到等于没写。

## 2. 设计原则（5 条）

1. **触发驱动**：同根因 ≥ 3 次 / 用户拍板 / Spec 才入库。初始卡片例外（来自 2026-06-08 多专家 review 的真实 blocker/major，有证据）。
2. **可执行 > 可读**：每条必须有真实代码片段 + grep/cargo 命令 + checklist。
3. **自维护**：复现一次 → 同 commit 内 `recurrence_count +1` + `log.md` 追行 + 卡 §5 追行。
4. **有生命周期**：drafting → active → merged（30 天零命中 + inline rules/）/ deprecated（场景消失）。
5. **scope 隔离**：trigger_paths 收窄到本仓 crates/；更广的规则进入根 `AGENTS.md`。

## 3. 在册约束索引表（与 AGENTS.md §4 对账）

| ID | Status | Severity | Title | 检测手段 | Recurrence | Last |
|----|--------|----------|-------|---------|------------|------|
| [HX-C001](./HX-C001-core-zero-runtime-dep.md) | active | blocker | helix-core 零运行时/零平台依赖 | `cargo tree`+grep gate | 1 | 2026-06-08 |
| [HX-C002](./HX-C002-async-send-boundary-no-lifetime.md) | active | blocker | async port Send 边界 + 状态禁 `&'a` | grep + 双端 check | 1 | 2026-06-08 |
| [HX-C003](./HX-C003-wasm-cfg-must-ci-typecheck.md) | active | blocker | Web 双目标代码须 native test + wasm final build，禁止 host runtime 泄漏 | 双目标 build/test/clippy + dependency tree | 2 | 2026-07-14 |
| [HX-C004](./HX-C004-features-additive.md) | active | major | features 必须 additive，禁互斥 | feature matrix build | 1 | 2026-06-08 |
| [HX-C005](./HX-C005-hotpath-zero-alloc.md) | active | major | 热路径零分配 O(1)（`&'static str` 不 String）| grep | 1 | 2026-06-08 |
| [HX-C006](./HX-C006-ffi-boundary-discipline.md) | active | blocker | FFI 边界铁律（no unwind / 单 owner / spawn_blocking）| grep + review | 1 | 2026-06-08 |
| [HX-C007](./HX-C007-effect-batch-across-boundary.md) | active | major | Effect/事件跨 Web/FFI 边界批处理，不逐条 | batch API + browser/FFI verification | 1 | 2026-06-08 |
| [HX-C008](./HX-C008-sync-pull-wire-contract.md) | active | blocker | sync/notify pull 契约：cursor 锚 nextSeq + 满批续拉 + mixed terminal 原子提交 | grep + 单测 | 2 | 2026-07-22 |
| [HX-C009](./HX-C009-expect-reconcile-loop-integrity.md) | active | major | 预期对账证据完整性（EXPECT 可证伪·DRIFT-PRED 如实记录·不阻断开发）| grep + 人工复核 | 2 | 2026-07-13 |
| [HX-C010](./HX-C010-deterministic-effect-ordering-from-collections.md) | active | major | 集合迭代→有序 Effect 生产必须确定性有序（有序集合/迭代前 sort，防 SipHash 随机序破可回放）| 回放测试 pin 顺序 + grep | 1 | 2026-06-18 |
| [HX-C011](./HX-C011-test-falsifiability.md) | active | major | 测试可证伪性铁律：压测/并发不变量测试必带「破坏即 fail」证伪 · 禁墙钟下界断言（结构化/计数证不变量）| grep wall>=断言 + 破坏→fail 证明 | 3 | 2026-06-18 |
| [HX-C012](./HX-C012-substantive-task-must-open-expect.md) | active | medium | 实质任务（含 test/CI/harness ≥50 行或新源文件）须开局写 🎯 EXPECT，禁事后补记（堵 DRIFT-PRED 永空 audit 后门）| grep DONE↔EXPECT 配对 | 3 | 2026-06-21 |
| [HX-C013](./HX-C013-workflow-agent-git-ref-guard.md) | active | major | Workflow/fork agent 会动主仓 git ref（checkout/merge/branch 删）——派发前后校验 HEAD/分支/ref + 分片即时 commit | 锚点对比 + reflog 巡检 | 2 | 2026-06-22 |
| [HX-C014](./HX-C014-branch-identity-in-session.md) | active | medium | 分支身份固化进 SESSION §0（当前分支+upstream+该不该删）——脚本自陈不靠记忆，杜绝「你在哪个分支」反复问 | `branch-identity.sh` + grep §0 唯一行 | 2 | 2026-06-23 |
| [HX-C015](./HX-C015-workflow-done-via-notification-not-poll.md) | active | medium | workflow/后台 agent 完成靠 task-notification 自动回灌，禁手工轮询；只外部不可追踪态才 /loop | 日志 grep「等…报告结果」计数↓ | 2 | 2026-06-23 |
| [HX-C016](./HX-C016-e2e-env-recovery-and-fast-harness.md) | active | major | 真Go e2e:WS hello 依赖 tp/gRPC 隧道会随时间断(恢复 playbook)+全UC单脚本双持久连接 wait_for(~3min)非LLM-workflow+快smoke vs精确per-UC取舍 | hello probe + 脚本 PASS/FAIL 表 | 2 | 2026-06-24 |
| [HX-C017](./HX-C017-distributed-performance-evidence-boundary.md) | active | major | 性能证据保持逐跳 correlation、确定性顺序与负载窗口边界 | 回放 + 分段指标 + 窗口检查 | 1 | 2026-07-13 |
| [HX-C018](./HX-C018-headless-semantic-interaction-boundary.md) | active | blocker | Headless 不是只吐 ViewModel：统一语义交互但禁止具体平台 UI | grep + 跨端 frame parity/负向破坏 | 1 | 2026-07-15 |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l` == AGENTS.md §4 表行数（都应=18）。不等 = 漂移，立即修。

## 4. 目录结构

```
docs/harness/
├── README.md            ← 本文（Layer B 索引）
├── TEMPLATE.md          ← 7 节卡片模板
├── decisions.md         ← 关键架构拍板记录（ADR-lite）
├── log.md               ← 跨卡复现日志
└── HX-C001 .. HX-C018   ← 18 条（全部 active）
```

## 5. 生命周期

```
drafting → active → merged（30 天零复现 + grep gate 稳定 + inline 进 AGENTS.md）
                  → deprecated（场景消失，如某 target 弃用）
```
晋升 merged checklist：§4 grep 命令存在 / CI gate `wc -l == 0` 放行 / §5 ≥1 条且最后一条 ≥30 天前 / 已 inline rules/ / frontmatter status=merged。

## 6. 何时深读母本

autonomous 任务开始前（协议 P1-P5）/ 第一次建卡（母本 §4+§5）/ 升级 merged·deprecated（§3.3）/ 周复盘自检（§8 闭环验证）。
