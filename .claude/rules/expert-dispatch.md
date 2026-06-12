# Rule — 专家 Agent / Skill 调度（渐进式披露）

> 这是根 [`CLAUDE.md`](../../CLAUDE.md) §4.6 注册表的完整展开（按需 Read）。
> 核心一句话：**能就近不加载，能加载不派活，能派一个不派编制——每升一级都要有触发条件。**

---

## 1. 渐进式披露四级阶梯（先低后高，禁止跳级浪费）

| 级 | 形态 | 成本 | 何时升到这级 |
|---|---|---|---|
| **L0 就近文件** | per-crate CLAUDE.md / `.claude/rules/` / `docs/harness/HX-C*` | 零（已在上下文或一次 Read） | 默认起点；规矩、命令、坑词典全在这层 |
| **L1 Knowledge Skill** | `/xxx-knowledge` 拉领域知识进**当前对话** | 轻（一次性注入，不开新上下文） | L0 答不了的领域问题：「这个机制怎么回事 / 业界怎么做 / 有什么坑」——只需要**知道**，不需要别人**干** |
| **L2 单 Agent** | `xxx-dev` 独立上下文执行 / 审查 | 中（独立 context + 工具） | 需要**第二双眼睛**（review diff）或**并行干活**（worktree 修复）；多文件 / 长链路 / 与主线无依赖 |
| **L3 Workflow 编制** | 多专家并行 + 对抗证伪 + 实测 | 重（10+ agents） | 重决策 / 重审查：架构方案过审、并发承压评估、P0 修复——**用户明示 workflow 或量级达标才用** |

**反模式**：单文件小修派 agent（L0 够）；问知识开 agent（L1 够）；没有对抗必要上编制（L2 够）。

---

## 2. 四专家注册表（领域 · 擅长 · 触发）

### rust-dev ↔ `/rust-knowledge`

- **领域**：Rust 语言与工程全栈——所有权/生命周期、类型系统（trait/GAT/对象安全）、错误处理、async（Send/Pin/tokio/atomics）、性能（零成本抽象/内存布局/分配/SIMD）、unsafe 治理、proc macro、workspace 设计。
- **helix 用途**：代码级热路径审查、并发正确性（本仓实战：E1 泵死锁链路逐行走查）、API/trait 设计权衡、clippy 护栏设计。
- **触发词**：借用报错 / Send bound / 性能回归 / unsafe 审查 / trait 设计争议。

### rust-tauri-skeleton-dev ↔ `/rust-tauri-skeleton-knowledge`

- **领域**：Rust + Tauri 2 桌面骨架端到端——IPC bridge（`#[tauri::command]`/invoke/emit/`im:__bus__` 单总线信封）、HTTP/WS 网络层（WsManager 状态机/重连退避/心跳）、SQLite 存储（读写分离/WAL/热路径 O(1)/多租户）、增量同步 cursor、热更新/窗口池。**参考真实仓 = cses-client（我们的现网）**。
- **helix 用途**：**现网对齐的唯一权威核验者**（本仓实战：揪出「12 命令实为 10」「写1读4实为三池」「too_long 不清表」）；M2 Tauri 四接缝接入主力；store:\*/imDataSource 隐式契约。
- **触发词**：现网怎么做的 / IPC 契约 / Tauri 接入 / message-v3 / DbManager / 灰度换引擎。

### rust-runtime-agnostic-core-dev ↔ `/rust-runtime-agnostic-core-knowledge`

- **领域**：一份 Rust core 编译 native + WASM + FFI——cfg/feature gating（additive）、executor-agnostic async（tokio vs wasm-bindgen-futures/spawn_local）、Send/!Send 边界、wasm 存储（sqlite-wasm-rs/OPFS/IndexedDB）、传输抽象（reqwest vs fetch、tungstenite vs web-sys WS）、FFI/uniffi/wasm-bindgen、wasm 体积优化。
- **helix 用途**：**三栖正确性的守门人**（本仓实战：揪出 wasm 无读池、grep 含 cursor 现场翻车、panic 策略互斥）；M3 wasm / M4 FFI 阶段的主力专家。
- **触发词**：wasm 编不过 / Send 冲突 / OPFS / feature 互斥 / FFI 边界 / 三端差异。

### dioxus-dev ↔ `/dioxus-knowledge` ⚠️ 项目约定：**只取 core 内核经验，禁渲染层话题**

- **可用领域**：VirtualDom step 循环、Mutation/Effect 批处理（sledgehammer 跨边界批量教训 = HX-C007 出处）、GenerationalBox/Copy 状态所有权（= ADR-002 出处）、Send/!Send 双形态存储（= ADR-001 出处）、调度与脏标记、corr/事件循环同构设计。
- **禁用领域**：RSX / 组件 / 渲染器 / dioxus-web/desktop UI 层——helix 不做 UI，提了就跑题。
- **helix 用途**：内核循环设计的对照审查（本仓实战：corr_map 不变量、批次边界、one-shot timer 与代码相反两处硬伤都是它抓的）。
- **触发词**：step 循环设计 / Effect 批处理 / 调度公平性 / 内核重入 / 借 Dioxus 踩坑史。

> 另有同名 `xxx-researcher` agent 族：仅当领域知识疑似过期（上游大版本/新机制）需要刷新知识库时用，日常开发不调。

---

## 3. 已验证的编制配方（L3，直接复用）

- **四专家并行审查**（架构文档 review / 承压评估两役验证）：四人各带 schema 出 findings → 每人 top-2 对抗证伪 → 主对话汇总修复。dioxus-dev 的 prompt 必须显式写「只用 core 经验，不涉渲染层」。
- **双线修复编制**（E1-E4 修复战验证）：按 crate 拆 worktree（白名单零交集）→ rust-dev 修 → 对抗 review → blocking 回炉 → 主对话 merge + gate.sh 终验。
- **压测随队**：凡涉性能结论，编制里加一个实测 agent（`/helix-stress-verify` skill 是它的操作手册）——无数字的性能主张不收。

## 4. 本仓自有 Skill（与专家 Skill 区分）

`/helix-new-module`（加业务模块 pattern）· `/helix-replay-test`（剧本测试写法）· `/helix-stress-verify`（承压验收）——这三个是**操作手册**（怎么在本仓干活）；专家 `/xxx-knowledge` 是**领域知识**（这个领域怎么回事）。先手册后知识。
