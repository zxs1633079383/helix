# Rule — 代码结构与领域驱动（Helix 项目级约束）

> 根 [`CLAUDE.md`](../../CLAUDE.md) §5 代码风格的完整展开（按需 Read）。
> 核心一句话：**单一职责 + 小文件 + 复杂度升序 + 设计模式服务于解耦；DDD 概念映射到 Helix 既有 sans-IO 六边形三层，不另起炉灶。**
>
> **优先级**：本文件是项目级 rule，与全局 `~/.claude/rules/common/coding-style.md`（「200-400 行典型，800 max」，仓外用户级文件）冲突时**本文件覆盖**（具体压一般，见 rules/README.md 优先级）。

---

## 0. 适用范围

适用全仓 `crates/**`。约束分**硬顶**（违反即拒）与**软目标**（尽量达成，超限要有理由 + 注释）。

| 约束 | 级别 |
|---|---|
| 单一职责 / 设计模式解耦 | 软目标（review lens） |
| 算法复杂度升序；热路径写 O(1) | **硬顶**（热路径，HX-C005）/ 软目标（其余） |
| `src/**` 源码单文件 ≤ 300 行 | **硬顶** |
| `tests/**` 单文件 ≤ 300 行 | 软目标（多独立用例可超，按关注点拆） |
| 每文件夹 CLAUDE.md + AGENTS.md 镜像 | **硬顶**（新建文件夹必须成对） |

---

## 1. 单一职责 + 单文件 ≤ 300 行

- **`src/**` 源码硬顶 300 行**。超限即拆：抽工具、按子职责裂成多文件，`mod.rs` 只 re-export（对齐 §5 many small files）。
- **判据**：一个文件只有一个「变更理由」。若两类需求会同时改它 → 拆。
- **`tests/**` 软目标 300 行**：测试文件常因「多独立用例聚合」合理超限；优先按**被测关注点**拆成多个 `tests/<concern>_test.rs`，而非硬塞一个。
- **docs/`.md` 不受行数约束**（本规则只管 `.rs`）。

### 存量债（已超 300 行的 `src/`，待拆 backlog，不阻塞新代码落地）

**🔒 baseline 机器强制（drift-review #1/#7 后门收敛）**：`scripts/structure-gate.sh` 的 WHITELIST 是
`path baseline_lines` 映射——**当前行数 > baseline 即 FAIL**（不再是「只比路径、注释里写只减不增」的
软规矩）。下表 = 当前基线（2026-06-22 锁定），**红线 = 不许再涨**；拆小到 ≤300 后从 WHITELIST 删行。

| 文件 | baseline 行 | 拆分方向（建议） |
|---|---|---|
| `helix-im/src/module.rs` | 432 | 本轮已把 send/query/sync/lifecycle/port reply 编排拆到 `send_flow.rs` / `query_dispatch.rs` / `sync_flow.rs` / `lifecycle.rs` / `port_reply.rs`；后续继续只保留 ImModule 壳、Module trait 路由与少量装配，压到 ≤300 后移出 WHITELIST |
| `helix-im/src/ws/registry.rs` | 783 | registry 骨架与各 handler 注册分文件 |
| `helix-driver-ffi/src/api.rs` | 733 | C ABI 导出按职责族裂分 |
| `helix-im/src/channel/gate.rs` | 579 | gate 三阶段（peek/persist/commit）可各成文件 |
| `helix-driver-host/src/storage.rs` | 578 | 按 StorageOp 族裂分（P6：`scan()` 加 additive 过滤/降序投影 +18）|
| `helix-driver-host/src/network.rs` | 463 | 按 port 实现职责裂分 |
| `helix-core/src/effect.rs` | 452 | Effect/Spec 族裂分 |
| `helix-im/src/parser.rs` | 430 | 临界，下次改动时收口 |
| `helix-driver-native/src/event_sink.rs` | 321 | 临界 |

> 注：`engine_loop.rs`（曾 ~1119）已拆至 168 行（FFI 生命周期解耦时），**已移出 WHITELIST 重新受 300 硬顶**。
> `helix-im/src/state.rs`（曾 299）已拆回 ≤300（当前 273），**已移出 WHITELIST 重新受 300 硬顶**。
> `helix-im/src/acl/to_effect.rs`（曾 680）已拆为 facade + `transport_effects.rs` / `storage_effects.rs`
> / `sync_http_effects.rs` / `projection_message_effects.rs` / `projection_control_effects.rs`，**已移出
> WHITELIST 重新受 300 硬顶**。
> 闸门接 §6；新代码必须 ≤ 300，存量超限文件「只减不增」由 baseline **机器断言**（先锁 A，实际拆分回正记
> ROADMAP B）。

---

## 2. 设计模式：服务于解耦，不为模式而模式

**目的是降耦合 / 高内聚 / 可测，不是堆模式**。守 CLAUDE.md §1.4 YAGNI——通用性靠「≥1 真实模块跑通 + 第二模块插得进」挣。

### 本仓已在用、推荐沿用的模式

| 模式 | 落点 | 解的耦合 |
|---|---|---|
| **Ports & Adapters / 六边形** | core 定 trait，driver 实现 | 业务 ↔ 运行时（HX-C001 单向依赖）|
| **Strategy（策略注册表）** | `ws/registry.rs` + `ws/handlers/*`（main 刚落地）| action → handler 分发，去掉巨型 match |
| **Repository / 仓储** | `Storage` port（通用 `UpsertSpec`/`GetSpec`）| 领域 ↔ 持久化；trait 不泄漏 IM schema |
| **State Machine** | gate 三阶段 · `WsRuntime` 连接态 | 非法状态不可达 |
| **Newtype / Type-state** | `ChannelId`/`Seq`/`ServerId`（Copy、owned）| make-invalid-states-unrepresentable（HX-C002）|
| **Tagged Union** | `InboundEvent` / `Effect` / `EventKind` enum | 一堆 Option/bool → 一个 enum |

### 反模式（禁，YAGNI 红线）

- ❌ 预建插件框架 / `inventory` 注册宏 / 命令中间件 —— 第二个真实模块出现前不抽
- ❌ 过度抽象的 EventBus trait 层 / 为单实现造接口
- ❌ 为「将来可能」加泛型参数 / 配置开关（边界要对，但不预建，见 §1.4）

---

## 3. 算法复杂度升序实现（从 O(1) 起）

**实现选型从最低复杂度起，先证明 O(1)/O(log n) 不可行才升级，并在注释留证据。** 对齐全局 `~/.claude/rules/common/performance.md` 热路径铁律。

| 路径类型 | 复杂度预算 | 级别 |
|---|---|---|
| **热路径写**（每事件命中：落库 / cursor 推进 / 计数）| **O(1)**：单 INSERT / UPDATE WHERE PK；**禁 read-modify-write** | 硬顶（HX-C005）|
| 查询读 | O(log n)：索引覆盖 WHERE 列 | 软（无索引即 full scan = 违规）|
| 分发 / 路由 | O(1) 查表（registry HashMap）优于 O(n) match 链 | 软 |
| 冷启动 / setup / migration | O(n) 可接受，注释写明 n 上界 | 软 |

**落地纪律**：写新 handler/schema 前过 performance.md §「设计前 checklist」；热路径写复杂度 > O(1) → 设计阶段就改 normalized，不留「先 ship 再优化」TODO。

---

## 4. DDD：映射到 Helix 既有三层，不另起炉灶

> ⚠️ **不要**新建 `application/` `domain/` `infrastructure/` 文件夹——会与 Helix 既有 sans-IO 六边形（业务模块 / core / driver 三层）打架。DDD 是**思维框架与命名对齐**，落点用既有结构。

### 概念映射表（真源）

| DDD 概念 | Helix 落点 |
|---|---|
| **Bounded Context 限界上下文** | 每个业务模块 crate（`helix-im` = IM 上下文；未来 presence / notify 各一个 crate）|
| **Ubiquitous Language 通用语言** | wire 契约术语对齐现网 Go（`post`/`posted`/`increment_channel`/`event_seq`）；行为真源 = cses-client（§4.5）|
| **Entity 实体** | 有 id + 生命周期：Channel（gate 持有）、Message |
| **Value Object 值对象** | newtype 不透明、owned/Copy：`ChannelId`/`Seq`/`ServerId`/`Id26`（HX-C002 禁 `&'a`）|
| **Aggregate (Root) 聚合根** | **channel gate** —— 封装 cursor 单调不回退不变量；外部只经聚合根改状态（gate 三阶段 peek→persist→commit）|
| **Domain Event 领域事件** | `DomainEvent` / `Effect::Emit`（不在 core 做 IO，吐 Effect）|
| **Repository 仓储** | `Storage` port（trait + 通用 `UpsertSpec`/`GetSpec`，不泄漏业务 schema）|
| **Domain Service 领域服务** | core 纯状态机 `handle`（同步纯函数，副作用只经 `out.push(Effect)`）|
| **Application 应用层** | `Module` trait 宿主 + Effect 编排 |
| **Infrastructure 基础设施** | driver 层（native / ffi）实现 ports |
| **Anti-Corruption Layer 防腐层** | `parser`（WS 帧 → 强类型 `InboundEvent`/`EventEnvelope`，边界零信任，HX-C 不变量 4）|

**铁律**：新业务模块 = 新 Bounded Context = 新 crate，只依赖 `helix-core` ports（不碰 tokio/sqlx）；领域逻辑进 core 纯状态机，IO 经 ports 出 driver。

---

## 5. 每文件夹 CLAUDE.md + AGENTS.md 镜像（渐进式披露）

扩展根 CLAUDE.md「指令文件同步铁律」到**所有有源码的文件夹**：

- **每个 crate 文件夹**（`crates/<crate>/`）必须有 `CLAUDE.md` + `AGENTS.md`，**内容完全一致**（AGENTS.md 是 codex 读的镜像）。
- **新建文件夹**（含子模块目录，按需）→ 成对创建；**改一个必须同步另一个**。
- 内容 = **就近局部指令**（该 crate/模块的不变量、命令、坑、文档索引），渐进式披露——根 CLAUDE.md 只挂索引，细节下沉到就近文件。
- 子目录级（模块级）CLAUDE.md 不强制全铺，但**一旦建就要配 AGENTS.md 镜像**。

### 存量缺口（镜像铁律当前已破 2 处，待补）

| 文件夹 | 缺 |
|---|---|
| `crates/helix-driver-host/` | 缺 `AGENTS.md`（有 CLAUDE.md）|
| `crates/helix-host-cli/` | 缺 `AGENTS.md`（有 CLAUDE.md）|

> 补法：`cp crates/<c>/CLAUDE.md crates/<c>/AGENTS.md`（内容一致即可）。

---

## 6. 闸门（建议，接入项目 gate 文化）

已落地 `scripts/structure-gate.sh`（与 `gate.sh`/`review-gap.sh` 同族，commit 1419690），建议挂 CI / pre-push 兜底：

1. **行数闸**：`src/**.rs` 超 300 行 → 检查（新增超限 = exit 1）。存量债走 WHITELIST，但 **WHITELIST 是
   `path baseline_lines` 映射、闸门读 baseline——当前行数 > baseline 即 exit 1**（机器强制「只减不增」，
   drift-review #1/#7 后门收敛；不再是软注释）。**⚠️ 后门第二形态（十三轮 drift-review 复发收敛）：把
   baseline 数字往大改 == 新增超限 == 等价绕 FAIL**——超基线唯一合法解是「测试外提 sibling / 拆文件」压回
   ≤baseline，**绝不允许上调 baseline 抹掉 FAIL**；baseline 只许往小改（拆分回正后锁更低值）。范本：
   state.rs 390（曾被 d022310 上调 baseline）→ `mod id26_tests` 外提 `state_tests.rs` → 340（baseline 锁 340）。
2. **镜像闸**：每个 `crates/*/` 必须有 CLAUDE.md + AGENTS.md 且 `diff` 为空（不一致 = exit 1）。
3. **分叉闸** `scripts/branch-sync-gate.sh`（同族，drift-review #2 收敛）：当前分支落后集成分支（dev-zlc）
   ≥5 commit = exit 1（把 SESSION 的散文「分叉监控」硬化成退出码，可挂 pre-push / Stop hook）。
4. 命中输出可 grep 的告警行，纳入 §4.7 drift-review lens。

---

## 7. 坑词典

| 反模式 | 危害 | 解 |
|---|---|---|
| 新建 DDD 三件套文件夹 | 与六边形三层打架，双套分层 | 映射进既有结构（§4 表）|
| 300 行一刀切套测试 | 多用例测试被迫塞一文件或假拆 | test 软目标，按关注点拆 |
| 「先 ship 再优化」热路径 | O(N) 数据翻倍即崩，迁移指数级 | 设计阶段 normalized（§3 + performance.md）|
| 改 CLAUDE.md 忘改 AGENTS.md | codex 读到过时镜像 | 成对改，§6 闸门兜底 |
| 为单实现造 trait / 预建插件框架 | YAGNI 违规，过度抽象 | 第二真实模块出现前不抽（§1.4）|
