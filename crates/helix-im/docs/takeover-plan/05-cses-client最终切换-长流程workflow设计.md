# 05 · cses-client 最终切换长流程 workflow 设计（spec-first）

> 状态：**设计 spec，未执行**（2026-06-23）。目标 = **一次性最终完全替换**：cses-client 退化为
> **纯 UI 渲染层 + Tauri 薄壳**，**所有 IM 业务逻辑终结在 helix 引擎**（收发/解析/cursor/落库/投影）。
> 保留现有 UI test case（加 helix 分支，不删 legacy，直到 100% 验证后）。每阶段**代码层验证无误**才进。
>
> 参照：上次全量 HTTP+WS 状态机经验（`gated-phase-migration` skill + CI-4 联调矩阵 `2026-06-23-…-CI4.md`）。
> 真源清单：cses-client `src/pages/message-v3/docs/14-helix接管前端改造清单.md` §2（2.1-2.5 分域）。

---

## 0. 为什么不能"今天直接 flip"（readiness 实况）

CI-4 验证 helix 引擎侧**零 bug**（V5-V8），CI-1~3 脚手架已落（git-dep + feature gate + 91 命令薄壳 +
`im:__bus__` 桥 + 前端 engine 开关）。但前端**只接了 ~30%**：

| 已接（CI-3，4 服务） | 未接（38 服务 dual-path 等切） |
|---|---|
| 发消息 `message.service.ts:169` · 读会话 `dialogSwitch.service.ts:471` · IPC 收帧 `ipcEvent.service.ts:98` + 归一化 | posts 写 7 · posts 读 17 · channel 写 17 · user-misc 11 · bot-agent 35 · 连接控制 2 |

**两个真阻塞**：
- **gap-4（BLOCKING）**：`im:channel:update*` 信号只携 `{channel_id}`（projection-schema 有意瘦身），前端
  **缺回读层** → helix 模式下 badge/会话列表不刷新。
- **38 服务未切** → 现在 flip `IM_ENGINE=helix` 会让这些服务的语义 invoke 落到「已 cfg-out 的 legacy 命令」
  或走 legacy HTTP 分支，行为分叉。

**结论**：**具备「跑长流程 workflow」的条件**（引擎已验、脚手架已固、逃生口 host-cli `cmd` 已通），
**不具备「立即 flip」**。本 workflow 的职责 = 把 38 服务 + gap-4 闭合，达到可 flip，最后一次性切。

---

## 1. 状态机（Phase=状态 · gate=转移边 · 红则停）

```
P0 前置基线 ──gate0──▶ P1 channel回读层(gap-4) ──▶ P2 posts写域 ──▶ P3 posts读域
   ──▶ P4 channel写+user-misc+bot-agent ──▶ P5 连接控制 ──▶ P6 全量flip ──▶ P7 删legacy(2周后)
        每条 ──▶ 都是一道 gate：代码层验证无误 + 现有 case 全绿 + 新 helix 分支 case 绿，红则停在本阶不前进
```

### P0 — 前置基线（entry gate，不写代码，只校验）
- helix CI-4 helix 侧绿（✅）；CI-1~3 已落（✅）。
- **git-dep 收紧（风险项）**：cses-client `src-tauri/Cargo.toml:142` 现 pin **移动分支** `feat/im-takeover`
  → cutover 期改 pin **具体 commit/tag**（防 dep 在脚下漂移）。
- `cargo build --features im_engine_helix` 过 + 前端 `tsc`/lint 过 + 现有 336 测试 legacy 路径全绿（基线）。

### P1 — channel 回读层（解 gap-4，BLOCKING 优先，对应 14-doc §2.1）
- 前端实现 `im:channel:update` / `im:channel:update-by-post` 信号处理：`channel_id` → 回读 dialogList
  → 还原 full DialogItem → 触发 badge/列表刷新。**回读在前端**（信号瘦身是 projection-schema 有意设计，
  前端按 id 回读权威行，不让 helix 塞半份 DialogItem）。
- **gate**：helix 模式 badge/会话列表刷新可见；现有 case 绿 + 新增「channel update → 回读 → 刷新」helix 分支 case。

### P2 — posts 写域（7 命令，14-doc §2.2）
- 给 7 个 posts-write 服务加 `isHelix()` 分支（urgent/confirm/cancel/quickReply/props/template/revoke）。
  helix 分支 = 语义 invoke + 订阅投影；legacy 分支保留不动。
- **gate**：7 服务 helix 分支 build + 现有 case 绿 + 每服务 1 条 helix 分支 case。

### P3 — posts 读域（17 命令 + 滚动，14-doc §2.3）
- 读服务加 helix 分支，走 `helixQueryBridge`（fire-ack + `im:messages:query_result` → Promise）。
- **gap-3（可后置）**：投影 `im_query_messages_by_channel` 缺 viewers/NOTICE 可见性过滤——
  本阶标记，低优先；若要 UI-only 彻底，过滤逻辑应入 helix 投影（非前端），列 P6 前的可选补丁。
- **gate**：读路径经 helix 返回正确数据；现有 case 绿 + helix 分支 case。

### P4 — channel 写 + user-misc + bot-agent（63 命令，14-doc §2.4）
- 三域剩余服务批量加 helix 分支。命名陷阱锚 CI-4 矩阵 §1.3/§1.5/§1.6（snake/camel/path param/Bot 鉴权）。
- **gate**：build + 现有 case 绿 + 抽样 helix 分支 case（每域 ≥2）。

### P5 — 连接控制（2 命令，14-doc §2.5）
- `im_reconnect`（helix gap-1 已修，`im:net:reconnect_requested` 控制信号）+ `im_read_channel`。
- **gate**：reconnect/read 控制路径经 helix 生效。

### P6 — 全量 flip（= 04 §2 的 all-in P1，不灰度）
- 此刻 42 服务全有 helix 分支 + gap-4 闭合。
- **切默认**：`engine_switch.rs` 默认 `Legacy→Helix`（或 boot.rs 默认开 feature + `IM_ENGINE` 默认 helix）。
- `cargo build --release --features im_engine_helix`。
- **全量验证（关键）**：336 测试在 **helix 模式**（非仅 legacy）全绿 + **CI-4 e2e 经 cses-client UI 重跑**
  V5-V8（不止 host-cli）+ host-cli `cmd` 逃生口对账。
- legacy 编译保留**≥2 周**（rollback `IM_ENGINE=legacy`，前后端成对，零数据丢失——共享 SQLite 表）。

### P7 — 删 legacy（2 周稳定后，UI-only 终态）
- 删 `src-tauri/src/features/im/`（~12.5k LOC）+ 各服务 legacy 分支 + `cfg(not(im_engine_helix))` 守护。
- cses-client = 纯 UI 渲染 + 薄壳；所有业务逻辑在 helix。**保留全部 test（已加 helix 分支）**。

---

## 2. 三条不可破铁律（贯穿所有 phase）

1. **保留现有 case**：每阶给服务**加 helix 分支测试**，**不删 legacy 测试**；直到 P7 才清 legacy。
   现有 case 是 parity 硬证——helix 行为必须 == legacy 行为（同输入同渲染结果）。
2. **代码层验证无误才进**：每阶 gate = `cargo build --features im_engine_helix` + 前端 build/lint +
   现有 case 零回归 + 新 helix 分支 case 绿。红则停在本阶，不前进（状态机铁律）。
3. **legacy 全程可回滚**：P6 前 legacy 默认；P6-P7 间 legacy 编译保留 2 周。任一阶段红 →
   `IM_ENGINE=legacy` 一键回退，零数据丢失（helix/legacy 同写共享表，schema 对齐 ADR-013）。

---

## 3. workflow 编排（`.claude/workflows/cses-client-final-cutover.workflow.js`，待材料化）

> **顺序状态机**（非并行 fan-out）——cutover 安全压倒速度，一阶红则停。各阶 agent 在 cses-client
> 仓（`worktrees/helix-integ`）干活，主对话读 gate 结论决定是否进下一阶。

```
meta.phases = [P1 channel回读, P2 posts写, P3 posts读, P4 channel/user/bot, P5 连接控制, P6 flip验证]
for (const phase of PHASES) {
  const r = await agent(`重构 cses-client ${phase.domain} 服务加 isHelix() 分支：${phase.detail}
    白名单：只改 ${phase.files}；不删 legacy 分支/测试；加 helix 分支 + 对应测试。
    gate：cargo build --features im_engine_helix + 前端 tsc/lint + 现有 case 全绿 + 新 case 绿。
    返回 {phase, gatePassed, filesChanged, casesAdded, regressions, blockers}`,
    { schema: PHASE_SCHEMA, effort: 'high' })
  if (!r.gatePassed) { log(`🔴 ${phase} gate 红，停`); break }   // 状态机铁律：红则停
  // gate 绿才进下一阶
}
// 全阶绿 → P6 flip agent：切默认 + helix 模式全量测试 + CI-4 e2e 重跑
```

- **agentType**：批量服务重构用默认子 agent（便宜）；P1 gap-4 回读层 + P6 flip 验证用 `rust-tauri-skeleton-dev`
  （现网对齐唯一权威）+ 随队 review。
- **验证随队**：每阶 gate 含真实 build + 测试，不接受「应该能行」无证结论（HX-C011 文化）。
- **可恢复**：resumeFromRunId——某阶红修复后从该阶续跑，已过阶段返缓存。

---

## 4. 开跑前置 checklist（P0 gate）

- [ ] commit review（architecture-drift-review）通过 + ledger 更新（helix 引擎侧无 drift）。
- [ ] git-dep pin 从移动分支 `feat/im-takeover` 收紧到具体 commit/tag。
- [ ] `cargo build --features im_engine_helix`（cses-client）过。
- [ ] 现有 336 前端测试 legacy 路径基线全绿（回归对照基准）。
- [ ] helix 侧 feat/im-takeover worktree 的 structure-gate 违规（to_effect.rs 786>763）+ 未提交脏文件收口。
- [ ] Go :8065 + 依赖（Redis/Bleve/admin/bot 按 P3/P4 域所需）就绪程度评估（纯前端重构阶段可不依赖 Go，
      flip 验证阶段需真 Go）。
</content>
