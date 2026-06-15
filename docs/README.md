# Helix 文档地图（docs/ 总索引）

> 全仓文档的**单一入口**：先看这页，再进具体文档。
> 解决「多份文档各自声称『定稿/权威』」的混乱——本页钉死**唯一权威链**与每篇文档的**状态/层级**。
> 维护规则见文末「§ 熵管理」。最近一次对账：2026-06-14。

---

## 一、唯一权威链（冲突时以高层为准）

| 维度 | 权威源 | 说明 |
|---|---|---|
| **设计理念 / 不变量（北极星）** | [`GOAL.md`](./GOAL.md) | L2 常量，每次会话开局必读；§1 设计理念 / §2 硬约束不可偏离 |
| **决策拍板（ADR）** | [`过会方案/helix-技术评审纪要.md`](./过会方案/helix-技术评审纪要.md) + [`harness/decisions.md`](./harness/decisions.md) | 纪要 = 7 条过会拍板；decisions.md = ADR-001..005 |
| **机制事实（IM 全链路）** | [`过会方案/helix-core-im-架构设计.md`](./过会方案/helix-core-im-架构设计.md) | 内核 + 四能力 + IM 模块 + 三端接入，2026-06-13 仍在更新 |
| **可执行护栏** | [`harness/`](./harness/README.md) | 7 张 HX-C 卡 = 把 GOAL §1/§2 编译成 grep/CI 闸门 |
| **代码本身** | `crates/helix-{core,im,driver-native,driver-wasm,driver-ffi}` | trait 签名 / effect 模型冲突时，**代码 > 任何设计稿** |

> 规则：**代码 > 机制权威源（架构设计）> baseline 设计稿（00-05）**；**纪要/decisions > 任何正文措辞**。

---

## 二、文档分层地图

### 北极星 / 进度层（常读）
| 文档 | 作用 |
|---|---|
| [`GOAL.md`](./GOAL.md) | 设计理念不变量 + 硬约束 + 成功标准 + 抽象高度边界（方向 A/B/C 红线） |
| [`ROADMAP.md`](./ROADMAP.md) | 天级 roadmap + TodoList 活账 + 验收 checklist + AI 工程化差距 |
| [`ROADMAP-full.md`](./ROADMAP-full.md) | 完整全景 roadmap（D+A 八阶段 + crux 构件映射 + headless 守卫逐阶段 + 性能自研层）；管「为什么这么排」，ROADMAP.md 管「这周做哪条」 |

### 过会方案/（评审 + 定稿层 · 有独立索引）
→ 进 [`过会方案/README.md`](./过会方案/README.md)（七篇主文档的阅读顺序 + 已拍板七条）。要点：
- **架构定稿** = `helix-core-im-架构设计.md`（机制权威）｜**决策存档** = `helix-技术评审纪要.md`（决策权威）
- **未来方向** = `helix-core-view-方向A设计.md`（core-view/ViewModel 投影，先 D 后 A）
- **Crux 参照** = `helix-crux-参照对照.md`（边界三分法：做到/故意不做/真缺口 + 还能从 Crux 取什么 8 向）
- **参考/历史** = `helix-Rust-Core-技术过会-回到原点版.md`、`helix-two-layer-architecture.md`（全量长稿，溯源用）

### 00-05 baseline 设计层（2026-06-08 冻结 · 溯源/细节用，非当前权威）
> 这一套是过会方案consolidation **之前**的逐 crate 详设草稿，统一冻结于 2026-06-11 基线入库。
> 仍被 ROADMAP 引用（如 `02-im-module-mount §4.4` transport 路由表设计），故保留；但**当前事实以代码 + 架构设计稿为准**。
> ✅ **命名已对齐**（2026-06-14）：本层原旧名 `cursor_upsert`/`CursorUpsert` 已**全量替换**为 `monotonic_upsert`/`MonotonicUpsert`（与代码 + 权威稿一致，见 [decisions.md ADR-004](./harness/decisions.md#adr-004)）；`grep -rn 'cursor_upsert\|CursorUpsert' docs/0[0-5]-*.md` = 空。

| 文档 | 范围 | 状态 |
|---|---|---|
| [`00-helix-architecture-final.md`](./00-helix-architecture-final.md) | 完整架构总稿 | **已被取代**（= two-layer-architecture 的前稿） |
| [`01-im-as-core-contract-demands.md`](./01-im-as-core-contract-demands.md) | IM 作为第一性需求方的 core 契约清单 | baseline |
| [`02-core-layer-contract.md`](./02-core-layer-contract.md) | helix-core trait/effect 契约草案 | baseline（签名以代码为准） |
| [`02-ddd-and-tech-selection.md`](./02-ddd-and-tech-selection.md) | DDD 限界上下文 + 三栖技术选型表 | baseline |
| [`02-driver-native.md`](./02-driver-native.md) | native 驱动设计 | baseline |
| [`02-im-module-mount-and-core-generality.md`](./02-im-module-mount-and-core-generality.md) | IM 挂载 + 通用性检验（ROADMAP §4.4 引用） | baseline · 半活 |
| [`03-driver-wasm.md`](./03-driver-wasm.md) | wasm 驱动设计 | baseline |
| [`04-driver-ffi.md`](./04-driver-ffi.md) | FFI 驱动设计 | baseline |
| [`05-tauri-host.md`](./05-tauri-host.md) | Tauri 宿主接入设计 | baseline |

### 专题 / 报告 / 参照
| 文档 | 作用 |
|---|---|
| [`三端运行/发消息全链路-三端.md`](./三端运行/发消息全链路-三端.md) | 发消息三端链路科普（机制详情见架构设计 §4.5） |
| [`AI工程化对齐-Bun-checklist.md`](./AI工程化对齐-Bun-checklist.md) | 对照 Bun Rust 重写仓的工程化差距（落地动作见 ROADMAP §四） |
| [`ops/gitlab-ci-runner-交接说明.md`](./ops/gitlab-ci-runner-交接说明.md) | GitLab CI 激活的运维交接（注册 docker runner / 镜像 / crates 镜像 / 资源）；配套根 `.gitlab-ci.yml` |
| [`reports/`](./reports/) | 修复战报（E1-E4 等），历史留痕 |
| `curx/` | Crux（方向 A 参照实现）调研笔记（WIP，未入库） |

---

## 三、harness（护栏层）
→ 进 [`harness/README.md`](./harness/README.md)。7 张 active 卡 HX-C001..C007 + [`OPERATING.md`](./harness/OPERATING.md)（什么事→调什么）+ [`decisions.md`](./harness/decisions.md)（ADR）+ [`log.md`](./harness/log.md)（踩坑流水）。

---

## § 熵管理（保持本地图不漂移）

1. **新增/删除 docs/*.md → 立即在本页登记一行**（含状态/层级）；新建文件不在表内 = 漂移。
2. **权威只有一条链**（§一）。任何文档不得再自称「定稿/权威」与架构设计/纪要并列——若是细节稿，写明「以代码/架构设计为准」。
3. **契约/命名 rename → 全量同步到所有文档（含 baseline 稿）+ 记 ADR**。旧名残留会让文档误导后来者，不留 churn 债。rename 后跑 `grep -rn '<旧名>' docs` 必须为空（ADR 里的变更溯源除外）。
   **2026-06-14 全树 doc↔code 对齐批次**（以 `crates/helix-core` 为真）：
   - `cursor_upsert`/`CursorUpsert(Spec)` → `monotonic_upsert`/`MonotonicUpsert(Spec)`（全树，记 [ADR-004](./harness/decisions.md#adr-004)）
   - `SetTimer` → `ScheduleTimer`（架构设计 / 方向A设计 图示）
   - `HttpClient` → `HttpRequester`（GOAL §0；trait 名以代码为准）
   - `Tick` 由「四种」更正为「**六种**」：四类核心（Command/Inbound/PortReply/Timer）+ 连接生命周期（Connected/Disconnected）（架构设计术语表/§2.2、大白话 §5.2）
   > 校验基线：`Effect` 7 variant（Send/Persist/PersistFire/Http/Emit/ScheduleTimer/CancelTimer，Render 属方向A未来）｜`Tick` 6 variant｜port 7 trait（Clock/EventSink/HttpRequester/IdSource/Random/Storage/Transport）｜Storage 5 method（batch_upsert/batch_update/get/monotonic_upsert/scan）。
4. **断链定期扫**：`grep -rno '](.*\.md' docs` 逐条验存在性（本次已清 `三端运行 → helix-架构选型-精简版.md` 死链）。
5. **空壳文件**（0 字节占位）要么填，要么删，不长期挂着。
