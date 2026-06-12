# Helix ↔ Bun（百万行 Rust 重写）AI 工程化对齐检查清单

> 2026-06-12 · 参照仓：`/workspace/ai-workspace/bun`（25 项机制实测调研）+ helix 基建实测审计。
> 图例：✅ 已对齐 · 🟡 有雏形需升级 · ❌ 缺失。每项给落地动作 + 量级。
> Bun 哲学一句话：**约束嵌进工具链，AI 犯错触发编译失败 / CI 红，而不是等人 review。**

## 第 1 层 · 指令层（AI 怎么知道规矩）

- [x] ✅ **1.1 per-crate 嵌套 CLAUDE.md**（Bun：root + 6 个子目录，就近继承）→ helix 仅 root 一份。动作：`crates/{helix-core,helix-im,helix-driver-native,helix-driver-wasm,helix-driver-ffi}/CLAUDE.md` 各一份薄文件（core：零依赖铁律+执行协议不变量；im：回放测试 pattern+ACL 约定+剧本写法；driver-native：worker/背压契约+压测命令；wasm/ffi：各自三栖坑）。量级：半天。
- [x] ✅ **1.2 AGENTS.md 镜像**（Bun：5 份，保证任何 agent 工具链都命中）→ 动作：root AGENTS.md 指向 CLAUDE.md 即可。量级：10 分钟。
- [x] ✅ **1.3 指令内嵌可复制命令模板**（Bun：CLAUDE.md 里直接贴 build/test 单命令）→ helix root §3 有三栖命令 🟡，但 per-crate 的「跑这一个 crate 的测试/压测」命令缺。动作：随 1.1 一起写。
- [x] ✅ **1.4 `.claude/settings.json` hooks 自愈管线**（Bun：PreToolUse 拦错误 build 路径、PostToolUse 自动 rustfmt）→ 动作：项目级 settings.json：PostToolUse Edit/Write `*.rs` → `cargo fmt -p <crate>`；PreToolUse Bash 拦 `cargo build --release`（本仓 release profile 是 wasm 体积向，压测才用）。量级：半天。
- [x] ✅ **1.5 行为真源声明**（Bun：Zig-as-reference，.zig sibling 是语义真源）→ helix 已实践「现网 cses-client 逐文件核验」。动作：已在实践，补一句进 root CLAUDE.md：「行为争议时 `cses-client/src-tauri/features/im` 对应 handler 为 source of truth」。

## 第 2 层 · 护栏层（AI 犯错 = 编译失败）

- [x] ✅ **2.1 clippy.toml deny-list**（Bun：29 条 disallowed-methods/types，每条带 reason 指向替代品）→ **本清单 ROI 最高项**。动作：`clippy.toml` + 各 crate lint 配置——helix-core deny `std::time::Instant::now`/`SystemTime::now`（→ Clock port）、`println!/eprintln!`（→ tracing）、`std::thread::spawn`；helix-im 同上 + deny `tokio`（双保险）；边界路径 deny `unwrap/expect`（clippy::unwrap_used 按 crate 开）。把 HX-C001/C005 从 grep 纪律升级为编译期契约。量级：1 天。
- [x] ✅ **2.2 rust-toolchain.toml 锁定**（Bun：nightly pin + components）→ 动作：pin stable 版本 + clippy/rustfmt components + wasm32 target。量级：10 分钟。
- [x] ✅ **2.3 rustfmt.toml**（Bun：限定 workspace members，vendor/test 例外）→ 动作：5 行默认配置即可，先有再调。量级：10 分钟。
- [x] ✅ **2.4 unsafe 治理**（Bun：每 unsafe 必配 `// SAFETY:`，43 条；缺注释编译报错）→ helix 实测 **3 处 unsafe、0 条 SAFETY**。动作：补 3 条 SAFETY 注释 + workspace 级 `#![warn(clippy::undocumented_unsafe_blocks)]`。量级：1 小时。
- [x] 🟡 **2.5 cargo public-api allowlist**（Bun 用 symbols.txt 管符号面；helix 纪要拍板 #2 早已定但未落地）→ 动作：`cargo public-api -p helix-core` 基线入库 + CI diff gate——业务概念混进 core API 面自动拦。量级：半天。
- [x] ✅ **2.6 Cargo workspace 信任边界**（Bun：108 crate 分层 bun_core→bun_sys→…）→ helix 5 crate 单向依赖 + `cargo tree` 闸门，**架构上已对齐**（规模适配）。
- [x] ✅ **2.7 cargo-deny / 依赖审计**（Bun 靠 vendor 治理）→ helix 无。动作：`deny.toml`（license + advisories + 重复依赖），低优先。量级：1 小时。

## 第 3 层 · 验证层（改了必被测到）

- [x] ✅ **3.1 测试分域隔离**（Bun：test/js/{bun,node,web}/ + regression/issue/N）→ helix tests/ 按 crate + 主题（replay/gate/send/lifecycle/proptest/stress/e4/e3）已分域。
- [x] ✅ **3.2 标准化测试 harness**（Bun：harness.ts 集中 bunEnv/tempDir/snapshot 工具）→ helix `tests/common.rs` 回放剧本辅助 + mock port，**同构已具**。可随 1.1 在 im/CLAUDE.md 写明「新测试必须用 common.rs 的剧本工具」。
- [x] ✅ **3.3 回归测试 issue 锚定**（Bun：regression/issue/${N} 路径强制）→ helix 无 issue 系统。动作：约定 `tests/regression_e{N}.rs` 命名（锚到待办清单编号，如 E1 死锁的回归测试），把已有 stress_engine 死锁场景标注 E1。量级：1 小时。
- [x] ✅ **3.4 bench 常态化 + 无数字性能主张自动拒**（Bun：42 域 bench + metrics 留痕）→ helix 压测三件套已入库 🟡 但是 #[test] 形态、数字只进 commit message。动作：criterion 化 + 基线 json 入库 + 「性能类 PR 必附 before/after」写进规则。量级：1 天。
- [x] 🟡 **3.5 CI 闸门链**（Bun：.github 多 job——format/lint/build/test 矩阵）→ **本地链已落地**（2026-06-12）：`scripts/gate.sh` 七道闸门（三栖 check → clippy → test → cargo tree gate → grep gate → harness 索引不变量 → public-api）+ `.git/hooks/pre-push` 调它 + PostToolUse rustfmt hook + clippy deny-list。**剩 GHA 远端 job 未做**（仓库还没远端，无紧迫）；接远端时 `.github/workflows/ci.yml` 跑同一条 `gate.sh`。量级：1 天（GHA）。
- [x] ✅ **3.6 快照/确定性验证**（Bun：toMatchInlineSnapshot + human review）→ helix 用「同剧本同 Effect 序列」的确定性断言 + proptest，同等强度。M3 追加三端 byte-level 一致比对（已在 ROADMAP）。

## 第 4 层 · 流程层（spec → 实现 → 验收成闭环）

- [x] ✅ **4.1 spec 先行**（Bun：proposal/issue 模板）→ helix 过会方案 7 篇 + 评审纪要拍板制 + ADR-lite（decisions.md），**强于 Bun 的对应物**（规模适配）。
- [x] ✅ **4.2 验收清单驱动**（Bun：Landing PR checklist 89 条）→ helix ROADMAP §三 checklist + 待办清单销账制。差一步：把「Landing 前过一遍 checklist」写成惯例（每次 merge 前我已在做，成文即可）。
- [x] ✅ **4.3 .claude/skills 任务库**（Bun：7 个 SKILL.md 微课程，@提及即加载）→ helix 的 docs/harness 7 卡是同构雏形 🟡 但不可被 agent 自动发现。动作：毕业三个起步 skill——`helix-new-module`（写第二业务模块的完整 pattern）、`helix-replay-test`（剧本测试写法）、`helix-stress-verify`（承压验收跑法）。量级：1 天。
- [x] ✅ **4.4 .claude/commands 工作流**（Bun：find-issues 等 6 个，定义「N 步精确流程」）→ 动作：起步两个——`/helix-acceptance`（跑全部结构+行为闸门并汇总）、`/helix-drift-check`（文档↔代码对账，本会话做过两次的流程固化）。量级：半天。
- [x] ✅ **4.5 结构化 commit + 模块 tag**（Bun：PR 纪律 + auto-close marker）→ helix 5 段 commit body + phase tag 规范已强于均值。
- [x] ✅ **4.6 对抗 review 流程**（Bun：Landing checklist 人审）→ helix 已 agent 化（双盲修复 + 对抗证伪 + 回炉），本会话实战 4 次拦下 7 条 blocking。固化为 4.4 的 command 更佳。
- [ ] 🟡 **4.7 版本契约文件**（Bun：LATEST + meta.json 增量构建对账）→ helix 暂无版本发布。动作：M2 收官时引入 `CHANGELOG.md` + workspace version 纪律即可，现在不做。

## 第 5 层 · 文档可发现性（AI 不用翻 wiki）

- [x] ✅ **5.1 机制文档随节标注现网对齐**（架构设计 17 图 + file:line 引用）→ 已对齐。
- [x] ✅ **5.2 per-module quick-start**（Bun：每模块 CLAUDE.md 内嵌 usage example）→ 随 1.1 落。
- [x] ✅ **5.3 术语表 / 大白话双梯度**（Bun 没有，helix 反超项）。

---

## 执行记录（2026-06-12 四批全部落地，单会话完成）

全部 ❌ 已清零。两项标 🟡 的收口条件：
- **2.5 public-api**：闸门逻辑已在 gate.sh [7/7]，`cargo install cargo-public-api` 后自动生成基线并 diff
- **3.5 CI**：本地版已生效（pre-push hook → scripts/gate.sh 七道）；仓库接远端后迁 GitHub Actions

## 原执行顺序建议（已执行，留档）

| 批次 | 项 | 产出 |
|---|---|---|
| 第一批（半天，立刻止血） | 2.2 + 2.3 + 2.4 + 1.2 | toolchain/rustfmt 锁定、3 条 SAFETY、AGENTS.md |
| 第二批（1.5 天，编译期护栏） | 2.1 + 2.5 + 3.5(本地 hook 版) | clippy deny + public-api 基线 + pre-push 闸门链 |
| 第三批（1.5 天，指令与课程） | 1.1 + 1.3 + 1.4 + 5.2 | 5 份 per-crate CLAUDE + hooks |
| 第四批（1.5 天，流程固化） | 4.3 + 4.4 + 3.3 + 3.4 | skills ×3 + commands ×2 + 回归锚定 + criterion |

完成判据：本清单 ❌ 全部转 ✅ 后，「2 人 + AI 主力」的开发形态与 Bun 百万行流程在机制层等价（规模项 2.6/4.7 按需缩放）。
