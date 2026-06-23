# cses 全量纯 helix 化 Round-2 — 双闸 conformance + 全代码铁律设计

> 2026-06-24 brainstorming 产出。Round-1(`2026-06-23-cses-full-helix-cutover-design.md`)首轮跑出
> gap-only/查清/NEED 偷懒档 → 本轮**纠偏**:除 search + createUser/status,**全部完整实现 + 双闸 conformance**。

## 0. 目标与 DONE

**目标**:剩余所有 HTTP+WS+UC **完整实现**(真代码,非骨架/gap-only/纯文档)+ **逐 UC 双闸 conformance** + review 已做的是否符合 UC。

**🔴 最高铁律(纠首轮偏差)**:每 UC(除排除项)**必须有完整可工作代码**——
- helix 缺命令 → **真实现**(禁写 NEED 文件交差)。首轮 NEED(todo 装配链 / sync-increment / usermisc-write)→ **全变实现任务**。
- cses 下放 → **真接线完整**(禁"查清无事/gap-only/纯文档结论")。首轮 U4同步/U5群管理/U10通知 "查清"档 → 重审 + 真缺即实现。
- **禁**:`todo!()` 骨架 / 自以为是的 gap 跳过 / 纯 docs 交差。
- e2e 验不了(无数据态)**也得完整代码 + 契约审计过**。

**唯一合法非实现**:① search(集合七)② createUser/status(Go 物理缺 handler)。**除此全部完整实现,无 exception。**

**DONE**:35 UC(37 − search 2)+ 全 HTTP + 全 WS,每条**全代码 + 契约审计过**;有数据态的再加 e2e 验。

## 1. 双闸(每 UC)

```
闸1 契约审计(always):
   helix 命令 body + endpoint + cses 下放 → 逐字对齐 full-map/partials UC 规格
   核:端点(posts/create 非 createPosts 等) · body 字段集(vs 真机curl真源) · 命名陷阱(camelCase/createAt/unReadPostId/SYSTEN)
闸2 行为 e2e(有数据态时):
   真 Go(444+64118eeb 自建群)跑该 UC 流程,WS 回声 action + 投影字段 + 渲染断言
判定:
   全代码 + 闸1 过           → ⏸ code-complete（e2e 缺数据未验,但代码完整契约对）
   全代码 + 闸1 + 闸2 过      → ✅ conformant-verified
   仅 search/createUser/status → 🔴 goBlocked
```

**台账无 gap-only/骨架/纯文档态。** 首轮 8 个 `✅ e2e-verified` 要补闸1 契约审计升级为 `conformant-verified`。

## 2. 工作分解(35 UC × 双闸)

### 2.1 先补 helix 缺命令(全代码,禁 NEED)
| 首轮 NEED | 本轮实现 |
|---|---|
| todo 装配链 | helix todo outbound + hello 收尾触发 + 投影(UC-10) |
| sync-increment | helix 增量 sync outbound 补全(UC-4) |
| usermisc-write | usermisc 写命令真实现(announcement 类已补,其余补;team 写无 caller 也实现命令) |
| 任何 partial(39/79 HTTP 缺的) | 逐条补 outbound 命令到全 |

### 2.2 cses 下放全代码(禁 gap-only)
重审首轮"查清"档 + 补:U4同步/U5群管理/U10通知 真接线;X-DB/U1/U2/U8 已实质下放→补契约审计闸。

### 2.3 review 已做的 conformance
首轮 8 验通 + 真下放未验档 → 逐 UC 闸1 契约审计,出偏差→修。

## 3. 执行(方案1 逐 UC 双闸 pipeline)

```
Phase 1 helix 缺命令全补(rust-dev·全代码·TDD·push main CI安全):
   todo/sync-increment/usermisc-write/HTTP partials → 真 outbound 命令,禁 NEED/骨架
Phase 2 cses 下放全代码(cses-client-dev·禁 gap-only):
   35 UC 逐个真接线 + 闸1 契约审计;热文件单 owner 串行
Phase 3 双闸验收(真 Go e2e + 契约审计):
   逐 UC 闸1(契约逐字) + 闸2(有数据态 e2e) → 台账三态
loop:台账非全 conformant-verified/code-complete → resumeFromRunId 续,直到 35 UC 全 code-complete+(可验的)verified
```

## 4. 验证 + 边界

- **agent prompt 硬禁**:无 `todo!()`/`unimplemented!()`/空骨架;无"前端零caller故跳过"自判 gap;无纯 docs 交差。每 UC 必须 diff 有实质代码或证明确无需改(契约审计证)。
- **完整性闸**:`fullmap-coverage.sh`(F2已修)三表 + 台账 conformant-verified/code-complete 计数。
- **契约真源**:full-map/partials + 真机curl真源.md + projection-schema.md。
- **隔离分支** feat/helix-takeover-integ;helix main push 走 CI 安全(core 没动+gate)。
- **诚实**:仅 search/createUser/status 可 goBlocked;其余全代码,e2e 缺数据如实标 code-complete(不假装 verified)。

## 5. 风险

- 35 UC 全代码 + 双闸 = 大工程多轮。但**禁偷懒**——每轮真代码进展,台账精确。
- 热文件(messageDialog 2891)改动大 → 单 owner + 强 jest。
- e2e 数据态:多数 UC 靠自建群 e2e;部分(已读需未读态/成员广播非self)缺数据 → code-complete 不强标 verified。
