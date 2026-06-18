# Helix 文档地图（docs/ 总索引）

> docs/ 的**单一入口**：先看这页，再进具体文档。
> **2026-06-17 大缩减（熵管理）**：55 → 29 个。删除 00-05 baseline 草稿 / 过会 presentation 稿（大白话/技术过会方案/完美过会指南/回到原点）/ two-layer 长稿 / 三端科普 / superpowers 临时稿；决策权威「评审纪要」7 条过会拍板已**并入 `harness/decisions.md`**；`eventbus` 两稿合一、`sync-v2` 两稿合一。目的：AI 广读的「叙事/设计」集压到 10 个，杜绝读到过时稿。
> 维护规则见文末「§ 熵管理」。最近对账：**2026-06-17**。

---

## 一、唯一权威链（冲突时以高层为准）

| 维度 | 权威源 | 说明 |
|---|---|---|
| **设计理念 / 不变量（北极星）** | [`GOAL.md`](./GOAL.md) | L2 常量，每次会话开局必读；§1 设计理念 / §2 硬约束不可偏离 |
| **决策（ADR + 过会拍板）** | [`harness/decisions.md`](./harness/decisions.md) | ADR-001..010 + **7 条过会拍板**（原「评审纪要」已并入，决策唯一事实源） |
| **机制事实（IM 全链路）** | [`过会方案/helix-core-im-架构设计.md`](./过会方案/helix-core-im-架构设计.md) | 内核 + 四能力 + IM 模块 + 三端接入；含 Effect 模型(Http/HttpFire) / BoundedSpawner / sync-v2 已验证 |
| **可执行护栏** | [`harness/`](./harness/README.md) | HX-C001..C009 = 把 GOAL §1/§2 编译成 grep/CI 闸门 |
| **代码本身** | `crates/helix-{core,im,driver-native,driver-host,driver-ffi}` | trait/effect 冲突时，**代码 > 任何设计稿** |
| **架构 review 台账** | [`review/workflow-review-ledger.md`](./review/workflow-review-ledger.md) | drift-review 收敛记录（§4.7 闸门） |

> 规则：**代码 > 机制权威源（架构设计）> 任何其它稿**；**decisions.md > 任何正文措辞**。

---

## 二、文档清单（29 · 按层）

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
| [`helix-sync-v2-设计.md`](./过会方案/helix-sync-v2-设计.md) | sync-v2 真 Go wire 契约 + 完成 spec（合并；**数据路径已真 Go 端到端验证**）。落地真相另见 `crates/helix-im/docs/sync-v2-能力与缺口状态.md` |
| [`helix-core-view-方向A设计.md`](./过会方案/helix-core-view-方向A设计.md) | 未来方向：core 加 `view()`/ViewModel 投影（先 D 后 A） |
| [`helix-crux-参照对照.md`](./过会方案/helix-crux-参照对照.md) | Crux 能力边界三分法 + 还能从 Crux 取的方向 |

### harness/（护栏层 · 按需 grep，非广读）
→ [`harness/README.md`](./harness/README.md)：HX-C001..C009（active 不变量卡）+ [`OPERATING.md`](./harness/OPERATING.md)（什么事→调什么）+ [`decisions.md`](./harness/decisions.md)（ADR + 过会拍板）+ [`log.md`](./harness/log.md)（踩坑流水）。

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
