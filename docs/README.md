# Helix 文档地图（docs/ 总索引）

> docs/ 的**单一入口**：先看这页，再进具体文档。
> **2026-07-13 熵减对账**：本页只维护**常读 canonical 入口**，不维护 Markdown 总量清单。已删除的 takeover/sync-v2 阶段稿、Superpowers 执行稿和 worker report 只在 Git 历史中保留，不再作当前索引。
> **2026-06-17 大缩减（历史记录）**：曾将广读叙事稿 55 → 29 个。该数字只代表当时 docs 入口收敛，不代表今天仓库 Markdown 总量。
> 维护规则见文末「§ 熵管理」。最近对账：**2026-07-13**（删除阶段稿后的索引收敛 + SessionStart UC 上下文真源）。

---

## 一、唯一权威链（冲突时以高层为准）

| 维度 | 权威源 | 说明 |
|---|---|---|
| **设计理念 / 不变量（北极星）** | [`GOAL.md`](./GOAL.md) | L2 常量，每次会话开局必读；§1 设计理念 / §2 硬约束不可偏离 |
| **决策（ADR + 过会拍板）** | [`harness/decisions.md`](./harness/decisions.md) | ADR-001..013 + **7 条过会拍板**（原「评审纪要」已并入，决策唯一事实源） |
| **机制事实（IM 全链路）** | [`过会方案/helix-core-im-架构设计.md`](./过会方案/helix-core-im-架构设计.md) | 内核 + 四能力 + IM 模块 + 三端接入；含 Effect 模型(Http/HttpFire) / BoundedSpawner / sync-v2 已验证 |
| **可执行护栏** | [`harness/`](./harness/README.md) | HX-C001..C016+ = 把 GOAL §1/§2 编译成 grep/CI 闸门；具体 active/archived 状态以 harness README 为准 |
| **代码本身** | `crates/helix-{core,im,driver-native,driver-host,driver-ffi}` | trait/effect 冲突时，**代码 > 任何设计稿** |
| **架构 review 台账** | [`review/workflow-review-ledger.md`](./review/workflow-review-ledger.md) | drift-review 收敛记录（§4.7 闸门） |

> 规则：**代码 > 机制权威源（架构设计）> 任何其它稿**；**decisions.md > 任何正文措辞**。

---

## 二、文档清单（canonical 入口 · 按层）

### 北极星 / 进度（常读）
| 文档 | 作用 |
|---|---|
| [`GOAL.md`](./GOAL.md) | 设计理念不变量 + 硬约束 + 成功标准 + 抽象高度边界（方向 A/B/C 红线） |
| [`ROADMAP.md`](./ROADMAP.md) | 天级 roadmap + TodoList 活账（P0/P1/P2）+ 验收 checklist |
| [`ROADMAP-full.md`](./ROADMAP-full.md) | 完整全景（D+A 八阶段 + crux 构件映射 + 性能自研层）；管「为什么这么排」 |
| [`COMMIT_CONVENTION.md`](./COMMIT_CONVENTION.md) | 提交规范（Conventional Commits + 5 段结构化 body + PR/Tag）；提交前必读 |

### 过会方案/（5 · 设计权威 + active 实现设计）
| 文档 | 角色 |
|---|---|
| [`helix-core-im-架构设计.md`](./过会方案/helix-core-im-架构设计.md) | ★**机制事实权威源**：内核 + 四能力 + IM + 三端 |
| [`helix-eventbus-设计.md`](./过会方案/helix-eventbus-设计.md) | EventBus PUB(单向)/REP(双向) 判据 + 下游能力契约（合并自 rep + 下游能力契约） |
| [`helix-sync-v2-设计.md`](./过会方案/helix-sync-v2-设计.md) | sync-v2 真 Go wire 契约 + 完成 spec（合并；**数据路径已真 Go 端到端验证**）。当前 UC/HTTP/WS/存储综合口径见 `crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md` |
| [`helix-core-view-方向A设计.md`](./过会方案/helix-core-view-方向A设计.md) | 未来方向：core 加 `view()`/ViewModel 投影（先 D 后 A） |
| [`helix-crux-参照对照.md`](./过会方案/helix-crux-参照对照.md) | Crux 能力边界三分法 + 还能从 Crux 取的方向 |

### harness/（护栏层 · 按需 grep，非广读）
→ [`harness/README.md`](./harness/README.md)：active/archived harness 的唯一索引 + [`OPERATING.md`](./harness/OPERATING.md)（什么事→调什么）+ [`decisions.md`](./harness/decisions.md)（ADR + 过会拍板）+ [`log.md`](./harness/log.md)（踩坑流水）。

### 模块级 spec / 执行记录（按需读，不能冒充当前 green）
| 文档族 | 当前路由 |
|---|---|
| `crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md` | **UC 消费视角真源**：command / projection / HTTP / WS / Go DB / Helix storage 一表对齐 |
| `crates/helix-im/docs/Strict Matrix/README.md` | **UC 发布证据入口**：契约、生成矩阵、远端存储真源与 fail-closed 判定 |
| `crates/helix-im/docs/uc-automation/` | `uc-command-set-spec.md` 是 UC/command 当前入口；`uc-test-order-spec.md` 是历史 DAG 参照 |
| `crates/helix-driver-ffi/docs/` | 移动端契约以 `移动端Command-Query-Event契约SPEC.md` 为主；QuickJS 接入文档是移动端联调说明 |
| `.superpowers/sdd/*.md` | worker brief/report/progress，只能作为执行线索和审查材料，不是 current green |
| `docs/sync-v2-report/*` / `docs/reports/*` | 历史审计/战报，保留 provenance；需要当前事实时回到代码、gate、collector 证据 |

### reports/（战报 + 性能留痕）
| 文档 | 作用 |
|---|---|
| [`helix-并发承压评估-2026-06-11.md`](./reports/helix-并发承压评估-2026-06-11.md) | **唯一实测性能 ground truth**（内核/gate/写放大数字 + E1-E10 瓶颈含开放项），缩减时移存于此 |
| [`fix-e1-e2-driver-native.md`](./reports/fix-e1-e2-driver-native.md) · [`fix-e3-e4-helix-im.md`](./reports/fix-e3-e4-helix-im.md) | E1-E4 修复战报 |
| [`2026-06-16-retro-持久化与merge.md`](./reports/2026-06-16-retro-持久化与merge.md) | 30 分钟复盘留痕 |

---

## § 熵管理（保持本地图不漂移）

1. **新增/删除 docs/*.md → 立即在本页登记**；新建文件不在表内 = 漂移。
2. **权威只有一条链**（§一）。任何文档不得再自称「定稿/权威」与架构设计/decisions 并列；细节稿写明「以代码/架构设计为准」。
3. **契约/命名 rename → 全量同步所有文档 + 记 ADR（decisions.md）**。rename 后 `grep -rn '<旧名>' docs` 必须为空（ADR 溯源除外）。
4. **断链定期扫**：`grep -rno '](\..*\.md' docs` 逐条验存在性。
5. **缩减优先级**：过会 presentation / baseline 草稿 / 科普稿一旦被权威稿覆盖即删（git 历史留底），不长期挂着误导 AI。
