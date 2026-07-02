# Spec — `helix-im/src` DDD 领域分包改造计划

> 状态：**设计定稿，待执行**（2026-06-30）
> 范围：仅 `crates/helix-im/src/` 目录结构重组 + 连带闸门/文档同步
> 铁律：**纯结构搬迁，零行为变更，不影响已有功能**

---

## 0. 现状诊断

`src/` 顶层堆着 **~50 个扁平 `.rs` 文件**，仅 4 个已成包：
`acl/` · `ws/`（+`handlers/`）· `outbound/`（+`bot_agent/`）· `channel_write/`。

扁平文件按职责天然归属若干**子域 / 聚合 / 六边形角色**，但当前全平铺，违背
根 `CLAUDE.md` §5「many small files，按 feature/domain 组织」与
`.claude/rules/code-structure-ddd.md` §1。

---

## 1. 设计原则（硬约束，全部遵守）

| 约束 | 来源 | 本计划如何遵守 |
|---|---|---|
| ❌ **禁建 `application/domain/infrastructure/` 文件夹** | code-structure-ddd §4 | 只按**子域 / 聚合 / 六边形角色**分包，复用既有 sans-IO 六边形三层语义，不另起 DDD 三件套 |
| **不影响已有功能** | 用户 | **纯移动 + `lib.rs` 门面 re-export**；零行为变更，跨 crate 与 crate 内调用点零改动 |
| 单文件 ≤300 行硬顶；`mod.rs` 只 re-export | code-structure-ddd §1 | 移动不改文件内容、不增行；新 `mod.rs` 仅 `//!` 文档 + re-export |
| 镜像铁律 CLAUDE.md + AGENTS.md | 根 §0 | 不强制每子目录铺 CLAUDE.md（既有子包亦未铺）；只更新 crate 级 CLAUDE.md/AGENTS.md 的 src 结构节，**成对改** |
| structure-gate WHITELIST 路径基线 | code-structure-ddd §6 | 移动后同步 `scripts/structure-gate.sh` 的基线路径 |
| HX-C001 core 零运行时 | 根 §0 | 纯移动无新依赖，不变量不受影响 |

### 1.1 两项关键取舍（已锁定，均取推荐项）

1. **内部 call-site 策略 = 门面兜底 · 零改动**
   `lib.rs` 根级 `pub use …as <旧名>`，crate 内对改名模块（`sync_session`/`query`/`send_flow` 等）
   的引用**一行不改**。diff 最小、最贴合「不影响已有功能」、风险最低。日后想清理可单开一轮。

2. **`channel_write/` 折入 `channel/` 包 → 成为 `channel/write/`**
   channel 聚合根（gate）与其写路径同处一包，聚合内聚最高；
   `helix_im::channel_write` 由门面 alias 保活。

### 1.2 TDD 先行执行口径（2026-07-02 用户拍板）

本计划虽是纯结构重排，执行 #9-#20 时仍必须逐 issue 走 **结构红测 TDD**：

1. 先写结构性检查（测试或 gate 断言），覆盖该 issue 的目标路径、旧路径清理、`mod.rs` 薄壳 re-export、必要白名单/文档镜像。
2. 迁移前运行该检查，必须 RED，且失败原因必须是"目标结构尚未落地"，不是语法错误或测试自身写错。
3. 再做该 issue 的最小文件移动、`mod` 声明和引用修正。
4. 运行同一检查变 GREEN，再跑该 issue 要求的 `cargo check` / `cargo test` / `structure-gate`。

禁止先搬文件制造编译红；禁止迁移后补测。#20 这类人工语义复核项，也要先写能自动失败的覆盖范围检查，再把人工结论写回文档/issue。

---

## 2. DDD 概念 → 目标包映射

| DDD 概念 | 目标包 | 收纳的扁平文件 |
|---|---|---|
| **Aggregate Root**（channel gate 三阶段不变量） | `channel/` | `channel.rs`(+tests) + 折入 `channel_write/` → `channel/write/` |
| **Domain State / Value Object** | `state/` | `state.rs`(+tests)、`correlation.rs` |
| **Anti-Corruption Layer（入站解析）** | `parser/` | `parser.rs`、`parser_post_props.rs`、`http_envelope.rs`、`base64.rs` |
| **Send 子域**（PendingSend 聚合 + 对账） | `send/` | `send_flow.rs`、`send_reconcile.rs`、`pending_send.rs` |
| **Sync 子域**（增量/会话/调度/心跳/回拉） | `sync/` | `sync_session.rs`、`sync_increment.rs`、`sync_scheduler.rs`(+tests)、`sync_flow.rs`、`pong_compensate.rs`(+tests)、`older_context.rs`(+tests) |
| **Read Model / CQRS 读侧** | `read/` | `query.rs`(+tests)、`query_dispatch.rs`、`query_dialog_render.rs`、`render_ready*.rs`(+tests ×3)、`read_relay.rs` |
| **连接生命周期** | `lifecycle/` | `lifecycle.rs`、`lifecycle_scan.rs` |
| **Application Host + 端口回复编排** | `module/` | `module.rs`(+tests)、`port_reply.rs`、`port_reply_emit.rs` |
| **通知/待办装配（UC-10）** | `todo/` | `todo.rs`(+tests) |
| 既有（不动） | `acl/` `ws/` `outbound/` | — |
| 留根 | — | `lib.rs`、`error.rs`、`schema.rs`、`commands.rs`（已是 outbound shim） |

> 映射依据 = code-structure-ddd §4「DDD 概念 → Helix 落点」真源表；落点用既有六边形结构，不另造分层目录。

---

## 3. 目标目录树

```
src/
├── lib.rs              # mod 声明 + 向后兼容门面(facade re-export)
├── error.rs  schema.rs  commands.rs        # 留根
├── channel/   mod.rs(←channel.rs) channel_tests.rs  write/(←channel_write/)
├── state/     mod.rs(←state.rs) state_tests.rs correlation.rs
├── parser/    mod.rs(←parser.rs) post_props.rs http_envelope.rs base64.rs
├── send/      mod.rs flow.rs reconcile.rs pending.rs
├── sync/      mod.rs session.rs increment.rs scheduler.rs(+tests) flow.rs
│              pong_compensate.rs(+tests) older_context.rs(+tests)
├── read/      mod.rs query.rs(+tests) dispatch.rs dialog_render.rs
│              render_ready.rs(+tests) render_ready_members.rs(+tests)
│              render_ready_replies.rs(+tests) relay.rs
├── lifecycle/ mod.rs(←lifecycle.rs) scan.rs
├── module/    mod.rs(←module.rs) module_tests.rs port_reply.rs port_reply_emit.rs
├── todo/      mod.rs(←todo.rs) todo_tests.rs
├── acl/  ws/  outbound/                     # 不变
```

---

## 4. 向后兼容门面（核心：保证「不影响已有功能」）

### 4.1 外部消费实测（grep 结论）

helix-im 之外（driver-ffi / driver-host / host-cli / benches）**只**消费这些路径：

- `helix_im::module::ImConfig`
- `helix_im::state::{ChannelId, Seq}`
- `helix_im::sync_session::{EventEnvelope, EventKind, PostFields}`
- `helix_im::outbound::canonical_command_name`
- `helix_im::channel::event_to_upsert_op`
- `helix_im::acl::to_effect::batch_upsert_events`
- 根 re-export `helix_im::ImModule` / `helix_im::IM_SCHEMA`

### 4.2 保活策略

- 目录名保持的（`module/ state/ channel/ parser/ outbound/ acl/`）→ **路径天然不变**，零改动。
- 唯一被外部用且改名的是 `sync_session` → `lib.rs` 加 `pub use sync::session as sync_session;`。
- crate 内部对改名模块（`sync_session`/`query`/`send_flow`/`sync_increment`/`sync_scheduler`/
  `older_context`/`channel_write` 等）的引用，同样靠 `lib.rs` 根级 `pub use … as <旧名>` 兜底
  → **call-site 零改动**。
- `#[path = "xxx_tests.rs"]` 测试 sibling 随源文件**同目录**搬迁，相对路径仍解析。

**净效果：helix-im 之外 0 行改动，crate 内 0 个 call-site 改动。**

### 4.3 计划新增的门面 re-export（lib.rs 根级）

```rust
// 向后兼容门面（DDD 分包后保活旧公共/内部路径，不影响已有功能）
pub use channel::write as channel_write;
pub use read::query as query;
pub use sync::{increment as sync_increment, older_context, scheduler as sync_scheduler, session as sync_session};
// send_flow / sync_flow / query_dispatch 等 crate 内部路径同法 pub(crate) use 兜底
```

> 注：具体 alias 清单以执行时 `grep crate::<旧名>` 与外部 grep 结果为准，做到「旧路径全保活」。

---

## 5. 连带改动（不可漏）

1. **`scripts/structure-gate.sh`** WHITELIST 基线改路径：
   - `crates/helix-im/src/module.rs 423` → `crates/helix-im/src/module/mod.rs 423`
   - `crates/helix-im/src/channel.rs 579` → `crates/helix-im/src/channel/mod.rs 579`
   - `crates/helix-im/src/parser.rs 430` → `crates/helix-im/src/parser/mod.rs 430`
   - `crates/helix-im/src/ws/registry.rs 783` → **不变**
   - ⚠️ 只改路径，**baseline 数字不许上调**（守 §6 棘轮后门收敛）。
2. **`.claude/rules/code-structure-ddd.md`** §1 存量债表的路径列同步（module/channel/parser/state）。
3. **crate 级 `CLAUDE.md` + `AGENTS.md`** 加「src 包结构」一节（**成对改**，内容完全一致）。
4. 每个新 `mod.rs` 写 `//!` 子域职责文档（就近渐进式披露）。

---

## 6. 分阶段执行（每 Phase 独立可编译 + 一 commit + tag）

> 对齐 `docs/COMMIT_CONVENTION.md` 模块切分：一 Phase = 一模块 = 一 tag。

| Phase | 内容 | 验证 |
|---|---|---|
| **P0** | 开局写 `SESSION.md §0` 🎯 EXPECT（可证伪断言见 §7） | — |
| **P1** | `state/`（+correlation）+ `channel/`（折 write） | `cargo check -p helix-im` |
| **P2** | `parser/`（+post_props/http_envelope/base64） | check |
| **P3** | `send/` + `sync/` | check |
| **P4** | `read/` + `lifecycle/` + `todo/` + `module/` | check |
| **P5** | `lib.rs` 门面 re-export + gate 脚本 + 2 doc + crate CLAUDE/AGENTS | 全量 |
| **P6** | 双端验收 + 三闸门 | 见下 |

P6 验收命令：

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test  -p helix-im
cargo tree  -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'   # 必空
scripts/structure-gate.sh    # 行数闸 + 镜像闸 + 分叉闸
scripts/gate.sh
```

每个 Phase/issue 的报告必须同时记录 RED 命令与 GREEN 命令；没有 RED 证据的 issue 不算完成。

---

## 7. 🎯 EXPECT（P0 写入 SESSION §0 的可证伪断言）

```
🎯 EXPECT DDD 分包仅动 crates/helix-im/src/** + scripts/structure-gate.sh + code-structure-ddd.md
+ helix-im/CLAUDE.md/AGENTS.md；外部 crate(driver-*/host-cli/core) 0 行改；
cargo check 四 crate 全过；cargo test -p helix-im 全绿；structure-gate.sh exit 0；
不新增 application/domain/infrastructure 目录 | 赌注 中 | 可逆 是
```

---

## 8. 风险与回滚

| 风险 | 缓解 |
|---|---|
| `#[path]` 测试 sibling 漏移 / `mod.rs` re-export 漏项 → 编译红 | 每 Phase 独立 `cargo check`，红则当 Phase 内修 |
| 门面 alias 漏一条 → 某旧路径解析失败 | P5 后跑全量 `cargo check` + `cargo test`，按报错补 alias |
| WHITELIST 路径漏改 → gate 误判新增超限 | P5 同步改 + P6 跑 `structure-gate.sh` 验证 |
| structure-gate baseline 被误上调（后门） | 只改路径不改数字，§6 已显式标注 |

- **回滚**：每 Phase 一 commit，`git revert <phase-commit>` 单 Phase 不破其余（模块自包含）。
- **零运行时不变量（HX-C001）**：纯移动无新依赖，天然保持。

---

## 9. 收尾终态行（P6 完成后写 SESSION §0）

```
✅ DONE helix-im/src DDD 分包(11 包·门面零改动) @<ts> | commit <起>..<止> | 6/6 Phase | main
```
