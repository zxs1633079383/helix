# 07 · Phase B 执行 spec —— cses-client 纯 helix 替换(无 dual-write)

> 状态：**执行 spec（先 spec 后并行实现）**，2026-06-23。05 文档是「dual-write→flip→删legacy」安全分级；
> 本文档按用户拍板**改为「直接纯 helix 替换」**——撤 `if(isHelix())else{legacy}` 双写,前端直调 helix。
> **安全模型变更**：不靠运行时 `IM_ENGINE=legacy` 回滚,靠**分支隔离**——全程在 `feat/helix-takeover-integ`
> (off main),用户线上用 main,改坏只影响分支;某域出问题 `git revert` 该域 commit(git 级回滚)。

---

## 0. 模式与安全(与 05 的差异)

| 维度 | 05(原 spec) | 07(本 spec·用户拍板) |
|---|---|---|
| 写法 | `if(isHelix()){helix}else{legacy}` 双写 | **纯 helix 直调,删 legacy 分支 + isHelix 守卫** |
| 安全 | 运行时 `IM_ENGINE=legacy` 回滚 2 周 | **分支隔离**(feat 分支 off main,不碰线上)+ git revert |
| flip/删legacy | P6 切默认 + P7 两周后删 | **合并**:替换即删 legacy,无独立 flip/删 phase |
| 速度 | 顺序(安全压倒速度) | **并行优先**(安全已由分支隔离兜,见 §4 约束) |

**净结果**:终态 = cses 纯 UI 渲染 + Tauri 薄壳,42 服务全走 helix,legacy IM 代码(~12.5k LOC)删除,
`IM_ENGINE` 脚手架退役。一步到位,不留 dual-write 中间态。

---

## 1. 基底盘点(feat/helix-takeover-integ @ 92fa2d291)

**已做(dual-write,需"转纯"= 删 else-legacy + isHelix 守卫)**：
- P1 channel 回读层(36ccd351e)· P2 posts 写 5(3aebb8c89)· P4a channel 写 16(bc379ed2a)
- read-family:posts 读 + user-misc 读(f333c17af)· git-dep 已指 main(92fa2d291)

**待建(纯 helix)**：
- posts 读「上拉续拉」(postContext 翻页 = 前端反复 invoke 推 anchor,见 takeover-plan 分析)
- user-misc 写(实查 0 可接,announcement写/permission/messageTop 已在 helix 后端补,前端接)
- **bot-agent 35**(全未接)· 连接控制 2(P5)· vote/score(第二网关,见 §5)

> ⚠️ 收口:worktree 当前 2 个未提交文件先归置(P0)。

---

## 2. 工作分解(域 → 文件 → 动作)

| 域 | 主文件 | 动作 | 已有? |
|---|---|---|---|
| D1 channel 回读 | ipcEventHandler/helixBusAdapter | 转纯(删 isHelix 守卫,helix 唯一路径) | dual ✅ |
| D2 posts 写(7) | message.service.ts | 转纯 + 补 announcement写×3(helix 后端已就绪) | dual 部分 |
| D3 posts 读(17+上拉) | message.service.ts | 转纯 + 上拉 postContext 翻页接缝 | dual 部分 |
| D4 channel 写(17) | message.service.ts | 转纯 + permission/messageTop(helix 已就绪) | dual 部分 |
| D5 user-misc(11) | message.service.ts + 其他 | 读转纯;写实查可接项 | 读 dual |
| D6 bot-agent(35) | bot 管理组件/service(message.service 外) | **新建纯 helix**(helix is_read 已补) | ❌ |
| D7 连接控制(2) | dialogSwitch/连接 service | 新建纯 helix(im_reconnect 已就绪) | ❌ |
| D8 vote/score | message.service.ts(vote 段) | **先扩 helix base 维度**再接(§5) | ❌ |

---

## 3. Gate(每域·红则停)

```
域 gate = ① cargo build --features im_engine_helix(cses src-tauri 编译过)
        ② 前端 tsc --noEmit + lint
        ③ 该域既有 jest case 绿(转纯后行为不变)+ 必要新 case
        ④ 真 Go/java e2e 抽样(P6 验证阶段集中做,见 §6)
红 → 停在本域,revert 或修,不带病进下一域。
```

---

## 4. 并行 vs 串行(硬约束:message.service.ts)

**约束**:`message.service.ts`(1517 行,31 isHelix 分支)是 D2/D3/D4/D5/D8 共享热文件 → **同文件并行编辑必撞**。

**策略(分两簇)**：
- **簇 A（message.service.ts 重·必串行 OR worktree 隔离后合并）**:D2/D3/D4/D5/D8。
  - 选项 A1 **串行 pipeline**:一域一域转纯(安全无冲突,慢)。
  - 选项 A2 **worktree 隔离并行 + 合并阶段**:每域独立 worktree 改 message.service.ts → 主对话合并
    (方法体在不同位置,git 多能自动合;冲突集中在 import/class-inject)。**并行但合并有成本**。
- **簇 B（独立文件·可真并行）**:D1(ipcEventHandler)· D6(bot 组件)· D7(连接 service)。这三域文件不重叠,
  Workflow `parallel` 真并行,零冲突。

**推荐**:簇 B 真并行(3 域同时)+ 簇 A 走 A1 串行(message.service.ts 一条线推),两簇并发跑。
→ 兼顾用户"并行"诉求 + 规避热文件冲突。vote(D8)因需先扩 helix base,单列在簇 A 末或前置。

---

## 5. vote/score 第二网关(P0 前置,helix 侧先动)

- **真相**:vote 是独立 java 服务 `localhost:3399`(pre = `api-pre.jinqidongli.com`),endpoint `/vote/*` `/average/*`,
  **非 /api/cses IM 子路由**。helix outbound `build` 只返相对 endpoint + 单 `api_base_url` → 表达不了。
- **解(takeover-plan/06 NEED 的 A/B/C 选一)**:推荐 **B = 命令带 gateway 标**(OutboundCommand 加
  `fn gateway()->Gateway{ Im | Vote }`,driver 据此选 base),改动最局部,不污染既有 IM 命令。
- **验证**:用真 createVote curl(路径换 `localhost:3399`)走真 java 验证;遇 gRPC + "unknown 地址"
  → 跑 `tp-connect.sh` + 重启 `/workspace/java/cses`(见恢复 playbook)。

---

## 6. 真验证(P6,集中做)

- 全域转纯后,在 **helix 模式**(此时无 legacy 可切,本就纯 helix)跑:cses 既有 jest 全绿 + CI-4 e2e 矩阵
  (89命令×8阶段)经真 Go/java 重跑 V5-V8 + host-cli `cmd` 逃生口对账。
- 真服务故障恢复:gRPC 错 + cses "unknown 地址" → `/workspace/java/zlc_ai/GenericAgent/tp-connect.sh`
  + kill/重启 `/workspace/java/cses`,会 retry,很快恢复,续验。

---

## 7. 铁律(贯穿)

1. **命名陷阱保真**:camelCase / `createAt`(非 createdAt)/ `unReadPostId`(大写 R)/ SYSTEN —— 后端权威字段名原样,前端不改写。锚 CI-4 矩阵 §1.3/1.5/1.6 + `真机curl真源.md`。
2. **绝不假装 wiring**:helix 缺命令/缺口 → 出 `NEED_*.md`,不造假分支。
3. **每域单 commit**(结构化中文 body)· core 零运行时不破 · 转纯后**既有 jest 行为不变**(parity 硬证)。
4. **gate 红则停**:编译/测试红 → revert 或修,不带病进下一域。

---

## 8. workflow 编排提案(待用户确认后材料化)

```
P0(主对话先做):① 收口 worktree 2 未提交 + git-dep pin 到 commit ② helix 扩 vote gateway base 维度(§5 方案B)+ localhost:3399 真验证
Phase B workflow（两簇并发）:
  簇B parallel:[D1 channel回读转纯, D6 bot-agent×35 新建, D7 连接控制新建]   ← 真并行(独立文件)
  簇A pipeline(串行):D2→D3→D4→D5→D8   ← message.service.ts 一条线,逐域转纯+补缺
  每域:cses-client-dev agent,gate(build+tsc+jest),红则停;单域 commit
  收尾:rust-tauri-skeleton-dev 对抗 review(现网对齐) + P6 真 Go/java e2e 抽样
```
- agentType:批量转纯用 cses-client-dev(现网对齐权威)+ 默认子 agent;P6 验证用 rust-tauri-skeleton-dev。
- 可恢复:resumeFromRunId,某域红修复后续跑。
