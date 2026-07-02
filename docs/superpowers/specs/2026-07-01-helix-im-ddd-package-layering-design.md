# helix-im / helix-core / driver 层 DDD 包分层规划（设计文档，纯规划·不涉及代码）

> 状态：用户已确认 / 待实施计划
> 范围：`crates/helix-im/src/**`（主体）+ `crates/helix-core/src/**`（确认无需改动）+ `crates/helix-driver-{host,native,ffi,wasm}/src/**`（确认无需改动）
> 非目标：不改任何 `.rs` 文件的实际逻辑；不拆/合 crate；不动 `ws/handlers/`、`acl/`、`outbound/bot_agent/` 内部结构；不涉及 loopforge-tauri-im / cses-client / cses-im-server / cses 等其他仓库。
> 本文档只产出**目标结构 + 完整影响面清单**；具体分几个 phase 落地、每个 phase 的执行顺序，留给下一阶段 `writing-plans` 决定。

---

## 0. 背景与动机

`crates/helix-im/src/` 当前 ~90 个源文件，除 `acl/`、`channel_write/`、`outbound/`（含 `outbound/bot_agent/`）、`ws/`（含 `ws/handlers/`）外，其余约 40 个文件全部平铺在 `src/` 根下（channel 生命周期、send 流程、sync cursor、query/渲染、todo、parser 各占一摊）。核心痛点：**按限界上下文找代码成本高**，且改动爆炸半径难评估（一个 `channel.rs` 摊在根目录，看不出它和 `channel_write/` 本是同一个聚合根）。

已确认背景事实（研究 agent 核验，详见对话历史）：
- 各候选限界上下文之间**无横向导入环**——按边界拆分在 Rust 可见性层面是安全的。
- `state`/`error` 是名副其实的共享内核（几乎所有文件都 import）。
- `channel_write/` 与 `sync_session.rs` 是被 4~13 个外部文件依赖的**枢纽**，但依赖方向单一（其它域 → 这两者，无反向），**域自持有**处理（见 §2.1）。
- `outbound/`（~30 文件）实际是 4 个子域（channel 管理 / posts / engagement / user_misc）+ 2 个 outbound 自己的局部共享内核（`registry.rs` 分发表、`send_build.rs` 共用发送体构造）混在一起。
- `helix-core` 与四个 driver crate **已经是干净的 DDD/ports-per-file 形态**，本轮不重新规划（见 §3、§4）。

---

## 1. 组织约定（贯穿全文）

1. **内核留在 crate 根**：被 3 个以上限界上下文 import 的低层原语（`state`/`error`/`schema`/`commands`/`http_envelope`/`base64`/`correlation`/`port_reply*`/`module`）不移动，继续平铺在 `src/` 根——这本来就是 Rust 惯例下的"共享内核"角色，不需要另建 `shared/`/`kernel/` 目录。
2. **每个限界上下文一个文件夹**，文件夹内 `mod.rs` **只做 re-export**（对齐 `.claude/rules/code-structure-ddd.md` §5 既有规则），实际逻辑落在同目录具名文件里——即使某个域当前只有 1~2 个逻辑文件，也统一遵守这条，保证心智模型一致（"进任意域文件夹，mod.rs 永远是薄壳"）。
3. **去除堆叠前缀（stutter）**：文件挪进同名域文件夹后，去掉与文件夹同义的前缀。例：`send_flow.rs` → `send/flow.rs`（而非 `send/send_flow.rs`）；`channel_change.rs` → `outbound/channel_admin/change.rs`。测试文件保留 `_tests` 后缀。
4. **枢纽文件域自持有**（不新建 `shared/`）：`channel_write/` 留在 `channel/` 域下（`channel/write/`），`sync_session.rs` 留在 `sync/` 域下（`sync/session.rs`）——它们被多个域依赖，这在 DDD 里是"核心域被多方读取"的正常形态，不因为"依赖方多"就被迫下沉到一个通用目录，那样反而丢失了"这就是 Channel 聚合根的写操作"这一领域含义。
5. **入站/出站对称边界**：`ws/`（含新挪入的 `ws/parser/`）是完整的**入站** ACL 管道（帧接收 → 解析 → 分发）；`acl/` 是**出站** ACL（Tick → Effect 翻译）。两者是一对镜像的边界层，本轮都不改内部实现，只是给 `ws/` 补上 `parser/` 子目录，让它和 `acl/` 结构对称。
6. **`outbound/` 保留自己的局部共享内核**：`registry.rs`（分发表）与 `send_build.rs`（共用发送体构造）不属于任何一个业务子域，继续留在 `outbound/` 根下，四个业务子域（`channel_admin/`、`posts/`、`engagement/`、`user_misc/`）作为它的子目录。
7. **命名判断**：原第 4 个 outbound 子域此前讨论中曾称"voting"，但实际文件（`vote_score*` + `quick_reply` + `urgent` + `template_received`）更准确的共同点是"消息互动类快捷指令"，本文档改用 **`engagement/`** 命名，供审阅时确认或改名。

### 1.1 TDD 执行口径（2026-07-02 用户拍板）

虽然本轮是**纯目录重排 + `mod` 声明调整**，仍按用户要求执行 **TDD 先行**。每个 issue（#9-#20）进入实现前必须先写一个结构性 RED 检查，再做迁移：

1. **RED**：为该 issue 新增或更新结构测试 / gate 断言，精确检查目标路径存在、旧路径不存在、`mod.rs` 只做 re-export、必要白名单/文档镜像已改；在迁移前运行，必须因"目标结构尚未落地"而失败。
2. **GREEN**：只做该 issue 允许范围内的文件移动、`mod` 声明、引用路径、gate/doc 同步，让同一个检查通过。
3. **REFACTOR**：仅清理 re-export/路径命名，不改变业务逻辑、函数签名、分支顺序。

禁止用"先搬文件制造编译红"替代 RED；那是编译破坏，不是 test-first。禁止等迁移完成后再补结构测试；那是 tests-after。若某个 issue 的验收项纯人工（如 #20 `coverage.sh` 语义复核），也要先写可失败的检查脚本/测试覆盖能自动判定的部分，再把人工结论写入对应文档或 issue note。

---

## 2. helix-im 目标目录树 + 完整文件映射

### 2.1 顶层结构总览

```
crates/helix-im/src/
├── lib.rs                                   # mod 声明 + re-export（内容随结构调整，逻辑不变）
├── module.rs (+module_tests.rs)             # 内核：ImModule 宿主
├── state.rs (+state_tests.rs)               # 内核：ImState
├── error.rs                                  # 内核：ImError
├── schema.rs                                 # 内核：wire schema/DTO
├── commands.rs                               # 内核：命令枚举
├── http_envelope.rs                          # 内核：envelope 辅助
├── base64.rs                                 # 内核：小工具
├── correlation.rs                            # 内核：corr_key
├── port_reply.rs (+port_reply_emit.rs)      # 内核：port 回复管道
│
├── channel/                                   # 限界上下文：Channel 聚合根
├── send/                                      # 限界上下文：乐观发送 + 对账
├── sync/                                      # 限界上下文：增量同步 cursor
├── query/                                     # 限界上下文：读侧投影/渲染
├── todo/                                      # 限界上下文：todo 域
├── outbound/                                  # 限界上下文族：出站命令分发
├── ws/                                        # ACL(入)：WS 入站管道（新增 parser/ 子目录）
└── acl/                                       # ACL(出)：Tick→Effect 翻译（不变）
```

### 2.2 `channel/`（聚合根：生命周期 + gate + 历史回填）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `channel.rs` | 539（已在 structure-gate 白名单，基线 579） | `channel/gate.rs` |
| `channel_tests.rs` | 144 | `channel/gate_tests.rs` |
| `lifecycle.rs` | 240 | `channel/lifecycle.rs` |
| `lifecycle_scan.rs` | 78 | `channel/lifecycle_scan.rs` |
| `older_context.rs` | 300 | `channel/older_context.rs` |
| `older_context_tests.rs` | 83 | `channel/older_context_tests.rs` |
| `channel_write/mod.rs` | — | `channel/write/mod.rs` |
| `channel_write/members.rs` | — | `channel/write/members.rs` |
| `channel_write/patch.rs` | — | `channel/write/patch.rs` |
| `channel_write/post.rs` | 257 | `channel/write/post.rs` |
| `channel_write/program.rs` | — | `channel/write/program.rs` |
| （新增）| — | `channel/mod.rs`（薄壳 re-export） |

### 2.3 `send/`（乐观发送 + 对账）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `send_flow.rs` | 115 | `send/flow.rs` |
| `send_reconcile.rs` | 134 | `send/reconcile.rs` |
| `pending_send.rs` | 157 | `send/pending.rs` |
| `pong_compensate.rs` | 111 | `send/pong_compensate.rs` |
| `pong_compensate_tests.rs` | 229 | `send/pong_compensate_tests.rs` |
| （新增）| — | `send/mod.rs`（薄壳） |

> `correlation.rs`（76 行，corr_key）留在内核，不进 `send/`——因为 `ws/handlers`、`acl/`、`send/` 都用它，属于跨域共享原语而非 send 专属。

### 2.4 `sync/`（增量同步 cursor 引擎）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `sync_flow.rs` | 201 | `sync/flow.rs` |
| `sync_increment.rs` | 188 | `sync/increment.rs` |
| `sync_scheduler.rs` | 213 | `sync/scheduler.rs` |
| `sync_scheduler_tests.rs` | 127 | `sync/scheduler_tests.rs` |
| `sync_session.rs` | 197 | `sync/session.rs` |
| （新增）| — | `sync/mod.rs`（薄壳） |

### 2.5 `query/`（读侧投影/渲染）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `query.rs` | 204 | `query/core.rs` |
| `query_tests.rs` | 264 | `query/core_tests.rs` |
| `query_dispatch.rs` | 66 | `query/dispatch.rs` |
| `query_dialog_render.rs` | 130 | `query/dialog_render.rs` |
| `read_relay.rs` | 95 | `query/read_relay.rs` |
| `render_ready.rs` | 112 | `query/render_ready/core.rs` |
| `render_ready_tests.rs` | 94 | `query/render_ready/core_tests.rs` |
| `render_ready_members.rs` | 124 | `query/render_ready/members.rs` |
| `render_ready_members_tests.rs` | 103 | `query/render_ready/members_tests.rs` |
| `render_ready_replies.rs` | 74 | `query/render_ready/replies.rs` |
| `render_ready_replies_tests.rs` | 71 | `query/render_ready/replies_tests.rs` |
| （新增）| — | `query/mod.rs`、`query/render_ready/mod.rs`（均薄壳） |

### 2.6 `todo/`（todo 域）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `todo.rs` | 125 | `todo/core.rs` |
| `todo_tests.rs` | 134 | `todo/core_tests.rs` |
| （新增）| — | `todo/mod.rs`（薄壳） |

### 2.7 `outbound/`（出站命令分发族）

局部共享内核（留在 `outbound/` 根，不进任何子域）：

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `outbound/registry.rs` | 298（白名单基线） | `outbound/registry.rs`（不变） |
| `outbound/registry_tests.rs` | 192 | `outbound/registry_tests.rs`（不变） |
| `outbound/send_build.rs` | 299 | `outbound/send_build.rs`（不变） |
| `outbound/mod.rs` | 61 | `outbound/mod.rs`（更新子模块声明） |

`outbound/channel_admin/`（频道管理类命令）：

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `outbound/channel_change.rs` | 283 | `outbound/channel_admin/change.rs` |
| `outbound/channel_change_dedicated.rs` | 244 | `outbound/channel_admin/change_dedicated.rs` |
| `outbound/channel_change_dedicated_tests.rs` | 166 | `outbound/channel_admin/change_dedicated_tests.rs` |
| `outbound/channel_create.rs` | 46 | `outbound/channel_admin/create.rs` |
| `outbound/channel_existing.rs` | 225 | `outbound/channel_admin/existing.rs` |
| `outbound/channel_member_admin.rs` | 85 | `outbound/channel_admin/member_admin.rs` |
| `outbound/channel_pinned.rs` | 158 | `outbound/channel_admin/pinned.rs` |
| `outbound/channel_read.rs` | 195 | `outbound/channel_admin/read.rs` |
| `outbound/channel_read_tests.rs` | 154 | `outbound/channel_admin/read_tests.rs` |
| （新增）| — | `outbound/channel_admin/mod.rs`（薄壳） |

`outbound/posts/`（帖子类命令）：

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `outbound/post_props.rs` | 38 | `outbound/posts/props.rs` |
| `outbound/posts_announcement.rs` | 182 | `outbound/posts/announcement.rs` |
| `outbound/posts_approval.rs` | 82 | `outbound/posts/approval.rs` |
| `outbound/posts_existing.rs` | 123 | `outbound/posts/existing.rs` |
| `outbound/posts_read.rs` | 216 | `outbound/posts/read.rs` |
| `outbound/posts_read_ext.rs` | 142 | `outbound/posts/read_ext.rs` |
| `outbound/posts_relay.rs` | 79 | `outbound/posts/relay.rs` |
| （新增）| — | `outbound/posts/mod.rs`（薄壳） |

`outbound/engagement/`（消息互动快捷指令 —— 命名待审阅确认）：

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `outbound/vote_score.rs` | 231 | `outbound/engagement/vote_score.rs` |
| `outbound/vote_score_tests.rs` | 220 | `outbound/engagement/vote_score_tests.rs` |
| `outbound/quick_reply.rs` | 35 | `outbound/engagement/quick_reply.rs` |
| `outbound/urgent.rs` | 102 | `outbound/engagement/urgent.rs` |
| `outbound/template_received.rs` | 36 | `outbound/engagement/template_received.rs` |
| （新增）| — | `outbound/engagement/mod.rs`（薄壳） |

`outbound/user_misc/`：

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `outbound/user_misc.rs` | 274 | `outbound/user_misc/core.rs` |
| `outbound/user_misc_helpers.rs` | 44 | `outbound/user_misc/helpers.rs` |
| `outbound/user_misc_tests.rs` | 207 | `outbound/user_misc/core_tests.rs` |
| （新增）| — | `outbound/user_misc/mod.rs`（薄壳） |

`outbound/bot_agent/`：**不动**（已独立干净，7 文件）。

### 2.8 `ws/`（ACL 入站管道）

| 旧路径 | 行数 | 新路径 |
|---|---|---|
| `ws/context.rs` | 42 | `ws/context.rs`（不变） |
| `ws/frame.rs` | 199 | `ws/frame.rs`（不变） |
| `ws/mod.rs` | 11 | `ws/mod.rs`（新增 `pub mod parser;` 声明） |
| `ws/handlers/*`（24 文件） | — | 不变 |
| `parser.rs` | 427（近白名单 430） | `ws/parser/core.rs` |
| `parser_post_props.rs` | 92 | `ws/parser/post_props.rs` |
| （新增）| — | `ws/parser/mod.rs`（薄壳） |

### 2.9 `acl/`：**不动**（已独立干净，11 文件）。

---

## 3. helix-core：确认无需改动

`crates/helix-core/src/`（`auth.rs`/`effect.rs`/`engine.rs`/`error.rs`/`lib.rs`/`module_host.rs`/`platform.rs`/`tick.rs` + `ports/{clock,event_sink,http,id,storage,transport}.rs`）已经是一港口一文件的干净 DDD/ports 形态，`ports/` 本身就是理想的子模块划分。唯一超限项 `effect.rs`（452 行）已在 `.claude/rules/code-structure-ddd.md` §1 存量债表追踪（"Effect/Spec 族裂分"），本轮**不重新规划**，继续按现有待办处理。

## 4. driver 层（host/native/ffi/wasm）：确认无需改动

driver 层组织原则是 **ports & adapters（一个 port 一个文件）**，不是业务限界上下文，和 helix-im 的 DDD 分域是两套不同层级的组织逻辑，不应强行套用同一套 inbound/outbound 拆分：

| driver | 入站→核心→出站编排枢纽 | 结论 |
|---|---|---|
| `helix-driver-native` | `engine_loop.rs`（168 行） | 已是独立文件，够干净 |
| `helix-driver-ffi` | `pump.rs` + `batch.rs`（HX-C007 批处理） | 已是独立文件 |
| `helix-driver-host` | 无专属 pump（`dispatch.rs`/`ops.rs` 是 StorageOp 分发/定义，非入站编排） | host 是给 native/ffi 共用的存储适配层，天然没有自己的入站泵 |
| `helix-driver-wasm` | `engine_loop.rs` | archived（ADR-006），不管 |

本轮对 driver 层的动作仅为：**确认已合规**，把已知"杂物文件"（host 的 `dispatch.rs`/`ops.rs`/`codec.rs`/`pools.rs`，ffi 的 `api.rs`/`batch.rs`/`vtable.rs`）记为后续代码拆分待办，**不重新规划目录**。

---

## 5. 影响面清单（docs / sh / workflow js 全量）

| 文件 | 当前引用方式 | 本轮处理 |
|---|---|---|
| `scripts/structure-gate.sh` WHITELIST 数组（9 条） | 硬编码精确路径 + 基线行数 | 路径按 §2 映射表整体改写；每条基线行数**不变**（只是路径变了，行数本身不受纯移动影响） |
| `.claude/rules/code-structure-ddd.md` §1 存量债表 | 与上面镜像的散文表 | 同步改写，保持既有镜像铁律 |
| `scripts/coverage.sh:69` `MATCH_KEY` 路径前缀剥离逻辑 | 路径匹配语义，非单纯字符串 | 🔴 标记为高风险项——实施阶段需人工复核该脚本的匹配语义是否因子目录嵌套而变化，不能只做查找替换 |
| `scripts/fullmap-coverage.sh:30,56` | 递归 `grep -r` | 🟢 低风险，目录挪动后自然存活 |
| `scripts/gate.sh:35` | grep `crates/helix-core/src/` 找业务词泄漏 | 🟢 不受影响（本轮不动 helix-core） |
| `scripts/cpp-smoke.sh:11` | `crates/helix-driver-ffi/smoke`（独立目录，非 src/） | 🟢 不受影响 |
| `.claude/workflows/helix-takeover-*.workflow.js`（5个）、`refactor-spec-then-build.workflow.js`、`cses-round2~7-*.workflow.js`（5个） | agent prompt 字符串里的字面旧路径 | 🟡 这些是**已执行完的历史迁移/一次性 workflow**，会静默过时但非活跃闸门；本轮列出但不强制修改，留作可选清理项 |
| `crates/helix-im/CLAUDE.md` + `AGENTS.md` | 散文描述模块布局 | **必须同步重写**，按 §2 新目录树逐域补充"职责 / 怎么用 / 依赖谁"，保持镜像铁律 |
| `docs/harness/HX-C*.md` | 泛化描述架构不变量，非硬编码路径 | 🟢 不受影响，无需改 |
| `docs/review/workflow-review-ledger.md` | commit-hash 驱动，无文件路径 | 🟢 不受影响；但本次重排若被执行，其提交量大概率触发 §4.7 的 10-commit drift-review 闸门，实施阶段需注意 |

---

## 6. 明确标记的后续代码工作项（本轮不做，仅记录）

- `ws/registry.rs`（633 行，全 crate 最大单文件，已在白名单基线 783）——目录重排是拆分它的天然契机，建议列为下一步优先代码工作项。
- `helix-core/src/effect.rs`（452 行）——按 Effect/Spec 族拆分，已有既存待办。
- `helix-driver-host` 的 `dispatch.rs`/`ops.rs`/`codec.rs`/`pools.rs`，`helix-driver-ffi` 的 `api.rs`/`batch.rs`/`vtable.rs`——分组讨论，本轮不动。

---

## 7. 验证 / 收尾清单（供未来实施阶段核对，非本轮执行）

实施阶段完成后必须成立：

- [ ] 每个 issue 均有结构性 RED→GREEN 证据：记录失败命令、失败原因、修复后通过命令
- [ ] `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` 全绿
- [ ] `cargo test -p helix-im` 全绿（纯移动+改 `mod` 声明，零逻辑变更，行为等价）
- [ ] `./scripts/structure-gate.sh` 通过（白名单路径已更新，镜像文档已同步）
- [ ] `crates/helix-im/CLAUDE.md` 与 `AGENTS.md` 内容一致且已更新为新模块地图
- [ ] `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 仍为空（确认重排未引入新依赖）
- [ ] 每个限界上下文文件夹的 `mod.rs` 均为薄壳 re-export，无实际逻辑残留
- [ ] `scripts/coverage.sh` 的路径匹配逻辑已人工复核并按需修正

---

## 8. 已收敛决策

1. `outbound/` 第 4 个子域命名采用 `engagement/`；如 code review 后有更准确命名，再在 #16 内局部调整。
2. 认可 `channel.rs` → `channel/gate.rs`、`parser.rs` → `ws/parser/core.rs`、`query.rs` → `query/core.rs` 等大文件改名；本轮只改路径，不改白名单基线数字。
3. 实施阶段按 #9-#20 issue 边界拆分，#9 起顺序推进；每个 issue 必须有结构性 RED→GREEN 证据和独立 commit。
