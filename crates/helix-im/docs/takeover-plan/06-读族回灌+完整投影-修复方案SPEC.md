# 06 · helix 读族回灌 + 完整投影（一次吐全·UI 直渲）修复方案 SPEC

> 状态：**spec-first 设计**（2026-06-23，用户拍板）。cutover workflow P3 撞出**两个真缺陷**，本 SPEC 定方案，
> 后续 workflow 按此**最优解**修复（autonomous）。两缺陷同根 = **「数据没一次吐完整给前端」**。

---

## 0. 两个缺陷（root cause，用户 review 实锤）

### 缺陷 A：读族 HTTP 响应被丢（前端拿不到数据）
**根因**（`module.rs:1017-1033`）：outbound 命令 `alloc_corr_internal()` → 建 `Effect::Http{corr}` → **但从不往
`corr_map` 注册 `CorrelationContext`**。HTTP 响应回来 `Tick::PortReply{corr}` → `handle_port_reply` 查
`corr_map[corr]` **查不到** → **静默丢弃**。

**为什么这么设计**：outbound 命令**读写混用同一条 fire-and-forget 路径**。对**写**命令合理——写的数据更新靠
**WS 回声**（post/post_update…→ 投影）拿，HTTP 响应只是 CommonRes ack，丢掉无所谓。但对**读**命令致命——
读**没有 WS 回声**，HTTP **响应体本身就是数据**，丢了 = 前端永远拿不到（top20/getReplies/bookmark/announcement
等 17 + user-misc 读 + bot-agent 读）。薄壳 `helix_passthrough_commands!` 返 `Result<bool>` ack 更坐实——
invoke 拿到的只是「入队成功」，不是数据。

### 缺陷 B：吐给 UI 的数据不够渲染，UI 又回查 DB（蠢代码·用户实锤）
**现状**（projection-schema §2 + P1 回读层 commit `36ccd351e`）：WS 信号类事件（channel update / post received）
helix 只 emit **三件套** `{channel_id,event_seq,msg_id}` 瘦信号 → **前端「按 id 回读本地 DB 行」再渲染**。
helix 明明刚把整行写进 DB（数据在手），却只吐个信号，逼前端**二次查库**。N 个渲染组件各查一次 = N 次冗余 DB 读。

**用户拍板**：**一次吐完整**——helix emit **完整可渲染 payload**（full DialogItem / MessageItem），前端**直接渲染，
零回读**。projection-schema §2「瘦信号防塞半份」的顾虑是**针对「部分数据」**；吐**完整数据**无覆盖问题 → 顾虑自解。

---

## 1. 修复 A：读族 request-response 回灌（复用 im_query 既有范式）

**不另起 transport**——沿用现网已验的 `im_query_messages_by_channel` + `helixQueryBridge` 范式（fire + 等
`im:xxx:result` emit + 超时）。

### 关联键铁律（review 补 · 防 corr/req_id 混用）
**两个不同的关联键，别混**：① `corr`（helix 内部 u64，`alloc_corr_internal` 生成，前端看不到，只用于 Http→PortReply 内部配对）；
② `req_id`（**前端 bridge 生成的唯一串**，放进 invoke 的 **payload**，用于前端把 `im:read:result` 配回是哪次 invoke）。
流程：前端 invoke(命令,{...,req_id}) → helix dispatch **从 payload 读出 req_id** → `corr_map.insert(corr, OutboundReadReply{req_id})`
→ fire `Http{corr}` → 响应 `PortReply{corr}` → handle_port_reply 查 corr 得 req_id → emit `im:read:result{req_id,body}`。
**并发同命令读**（两频道同时 top20）靠 **req_id 唯一**区分——req_id 必须前端每次 invoke 唯一。**禁拿内部 corr 当 req_id**（前端拿不到 corr）。
读命令须走 **mandatory `Effect::Http`（带 corr、有 PortReply 回路）**，不是 fire-and-forget 的 HttpFire。

### helix-im 侧
1. `state.rs` 加 `CorrelationContext::OutboundReadReply { req_id }`（req_id 来自命令 payload，见上铁律）。
2. `module.rs` outbound 分发：**区分读 vs 写**（registry 给每命令标 `is_read: bool`，读族 = 无 WS 回声那些）。
   - **读命令**：alloc corr → `corr_map.insert(corr, OutboundReadReply{req_id})` → push `Effect::Http{corr}`。
   - **写命令**：保持现状（fire-and-forget，不注册 corr context，数据走 WS 回声）。
3. `handle_port_reply` 加 `OutboundReadReply` 臂：拿 HTTP 响应 `outcome`（`http_envelope` 解出
   `{status,headers,body}`）→ **emit `im:read:result{req_id, body}`，body = 后端响应体原样透传**（**后端返回什么
   helix 返什么**，零本地改业务数据，守 HX-C005 不在热路径瞎序列化）。
4. registry `build` 给读命令的 Http 也带 corr（已带），只是新增「读才注册 context」分支。

### cses-client 侧
- 读服务用 bridge（仿 `helixQueryBridge`）：`invoke(命令, {…, req_id})` + 等 `im:read:result{req_id}` → resolve
  完整 body → **前端直接渲染**（透传数据，本就完整）。超时 5s 兜底。

### 不变量
- 读 = 透传（后端→helix→前端，body 不改）；写 = WS 回声（不动）。HX-C001 core 仍零运行时（仅 helix-im 加
  context 变体 + 回灌 emit，无新 Effect 变体；emit 走既有 `Effect::Emit`）。

---

## 2. 修复 B：完整投影（fat emit，杀「瘦信号+前端回读」）

**原则**：helix 在**写完行的那一刻数据在手** → **从手上的数据直接 emit 完整可渲染 payload**（零额外读），前端直接渲染。

🔴 **热路径回读铁律（review 收紧 · 防反噬）**：「完整投影」**不等于**「emit 前先 server 端回读 DB」。
sans-IO 下「回读」= 发 Storage 读 Effect → 异步回来 → 再 emit = **每事件一次 DB 读**。若用在**高频事件**
（每条进来的消息 / post received / sync 增量逐条）上，等于把「前端 N 次回查」搬成「helix 每事件回读」——
**性能问题没消除，只换了地方，还碰 HX-C005 热路径**。所以：
- ✅ **默认且唯一正道**：emit 用**写入时手上的完整数据**（写 Effect 的 payload 即源数据，直接拿来 emit，零读）。
- ⚠️ **server 端回读仅限**：① 低频、非每事件命中（如用户切会话一次性拉取）② 且数据确实不在手必须 join。
  **绝不上「每条消息 / 每条 sync 增量」的热路径**。命中这种 → **重构让写入时就捕获该字段**，而不是加一次读。
- 判据：这个 emit 在「每事件命中」的写路径上吗？是 → 必须数据在手直接吐，**禁回读**。

### 改动
1. **projection-schema.md §2 改契约**：信号类从「三键瘦信号 + 前端回读」→「**完整 payload（full DialogItem /
   MessageItem）**，UI 直渲」。逐 channel 重列**完整字段集**（守命名陷阱 §6：unReadPostId 大写R / SYSTEN /
   createAt / camel vs snake）。
2. **helix-im emit 工厂**（`to_effect.rs` / `to_effect_s1.rs` / `query.rs`）：信号类工厂用**写入时手上的完整数据**直接吐完整行（零额外读）。**热路径禁回读（见上铁律）**——若发现某工厂"没数据必须回读"且在每事件热路径上，**改写入路径让它捕获该字段**，不要加读。
3. **强制对账测试**（`projection_schema_test.rs`）：断言每信号 channel 的 `data` 键集 == **完整可渲染集**
   （不再是三键），缺字段即 fail（HX-C011 可证伪）。
4. **回正 P1**（commit `36ccd351e`）：cses-client 的「按 channel_id 回读 dialogList」回读层 **删除/降级**——
   helix 既吐完整 DialogItem，前端 dispatcher 直接 next，无需回读。P1 的回读层从「主路径」降为「兜底」或删。

### 边界（防过度）
- **真正只需信号、前端本就有完整态**的事件（如纯 cursor 推进无渲染变化）保持瘦——不强行 fat。判据：**前端渲染
  该事件是否需要 DB 里的完整行**？需要 → fat emit；不需要 → 信号即可。逐 channel 在 projection-schema 标注。

---

## 3. 受影响范围审计（workflow Phase 0 必做：调用检查 + 综合已完成/未完成）

**全 89 命令三分类**（registry 加 `kind: Read|Write|Control` 标注，作 Phase 0 产物）：
| 类 | 命令 | 数据回前端的路 | 本 SPEC 动作 |
|---|---|---|---|
| **Write** | posts 写7 / channel 写14 / 部分 bot | WS 回声 → **fat emit**（修复 B）| 投影吐完整 |
| **Read** | posts 读17 / user-misc 读 / bot-agent 读 | **HTTP 响应 → 回灌 emit**（修复 A）| 加 read-reply context |
| **Control** | reconnect / read 已读 | 控制信号 / WS | 不涉数据回灌 |

**已完成接口 review（修复前必查）**：
- **P1** `36ccd351e`（channel 回读层）：含缺陷 B 的「前端回读」蠢模式 → **回正为 fat emit**。
- **P2** `3aebb8c89`（posts 写 5）：查这 5 个写命令的**投影是否 fat**（post_update/post received emit 是否完整）→
  不完整则补 fat emit。
- **P3 未完成**（读族 0 分支）：本 SPEC 修复 A 落地后**重跑**——读服务经 bridge 接通。

---

## 4. P2 两 blocker 不许「YAGNI 跳过」——查清再定

`im_urgent_cancel` / `im_update_post_props` agent 报「前端无 caller」→ **不许直接跳**。Phase 0 必须：
1. **grep 现网前端**确认是否真无 caller（message-v3 + 组件层 + src-tauri legacy）。
2. **若真无 caller**：确认 helix 侧该命令**实现完整且最优**（endpoint/body 锚真源、就绪待接），文档写明「helix
   已就绪·前端暂无入口·非缺口」+ 列出未来接入点——**这才是合格收口，不是「跳过」**。
3. **若有 caller**（agent 漏看）：立即按修复 A/B 接线。
4. `im_search_do`（Go 未实现·Java 全局搜索遗留）：留 legacy + 记 blocker（**唯一合法「暂不切」**，因对端缺）。

---

## 5. 最优解硬约束（修复全程守）

- ❌ 禁「helix 吐半份 → UI 回查 DB」（缺陷 B 蠢模式）→ ✅ 一次吐完整，UI 直渲。
- ❌ 禁读族 fire-and-forget 丢响应（缺陷 A）→ ✅ 读族 HTTP 响应回灌前端（后端返回什么吐什么）。
- ❌ 禁「有 blocker 就 YAGNI 跳」→ ✅ 查清：对端缺(记)/前端无入口(就绪待接)/漏看(立即接)。
- 守 HX-C001（core 零运行时）/ HX-C005（热路径不瞎序列化，回灌走 emit 既有路）/ HX-C011（投影对账可证伪）。
- 读族透传**不在 helix 改业务数据**（后端权威），但**完整**（一次给够 UI）。

---

## 6. workflow 编排（spec-first·autonomous·并行）

```
Phase 0 [审计·串行]：89 命令三分类 + registry 标 kind/is_read + review P1/P2 commit 找缺陷B + 查 P2 两 blocker caller
   → 产出受影响接口清单 + 已完成/未完成矩阵
Phase 1 [helix core·并行A]：修复 A——CorrelationContext::OutboundReadReply + handle_port_reply 回灌 emit + registry is_read
Phase 2 [helix core·并行B]：修复 B——projection-schema §2 改完整契约 + emit 工厂 fat + 对账测试改完整集
   （Phase 1/2 改不同文件区，可并行；rust-dev 随队 review 正确性）
Phase 3 [回正]：P1 回读层降级/删（依赖 Phase 2 fat emit 落地）
Phase 4 [cses-client·并行]：读服务接 bridge（依赖 Phase 1）+ 写服务核实 fat 渲染（依赖 Phase 2）
Phase 5 [P2 收口]：两 blocker 按 §4 定论（接线 or 就绪文档化）
每 Phase gate：cargo test -p helix-im/core + cses tsc/lint/jest + 守命名陷阱 + 编译过才 commit·红则 revert
```
> Phase 1/2 是 helix 核心（rust-dev / rust-tauri-skeleton-dev 把关正确性）；Phase 4 cses-client-dev 接线。
> **不做 P6 flip**（仍留用户确认）。
</content>
