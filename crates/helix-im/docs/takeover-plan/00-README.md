## helix-im 全量接管 HTTP+WS 长链路方案 — 文档集 README

> **北极星**：让 helix-im(Rust) 全权接管 cses-client `src/pages/message-v3` 的 **HTTP 出站 + WS 入站 + 业务编排**；Angular 退化为**纯渲染层**（业务零下沉到前端）；前后端经 Tauri IPC（`im:__bus__` 单总线 + 语义 invoke 命令）通信。后端真源 = `mattermost/server/channels/csesapi`；前端真源 = `cses-client`；现状 helix = `crates/helix-im/`（sync-v2 ~90%）。
>
> **硬约束**：全程**不含**任何 bitmap / segment / channel_bitmap（ADR-013 下线）。`segment_id` 死列已移除，不纳入设计。

### 一图流（状态机式推进）

```
P0 基线 → P1 SendPosts编排 → P1.5 出站横切层 → P2 outbound-registry化+posts写 →┐
  ┌──────────────────────────────────────────────────────────────────────────┘
  └→ [P3a posts读族 ∥ P3b channel写+P0-2系统消息] → P4 WS全量19事件+SyncScheduler →┐
  ┌────────────────────────────────────────────────────────────────────────────┘
  └→ [P5a user-misc ∥ P5b bot-agent] → P6 Tauri四接缝 → P7 cses-client接入(只设计)
```
节点 = Phase 状态；箭头转移条件 = 该 Phase 验收 gate **全绿**（涉及接口/事件 100% 测试通过 + 覆盖率达标）；gate 红则**不转移**。`∥` = 文件白名单零交集 + 无数据依赖 → 可 worktree 并行（同 parallelGroup）。

### 文件索引

| 文件 | 内容 |
|---|---|
| `00-README.md`（本文件）| 总览 + 北极星 + 文件索引 + 状态机一图流 |
| `01-接管架构与状态机.md` | helix 三层替换 `src-tauri/features/im` 的架构 + 数据流全景 + 四正交子状态机（连接/同步含 SyncScheduler/发送/接管推进）mermaid |
| `02-分阶段路线.md` | 状态机式有序 Phase 推进图 + 每 Phase（scope/后端参考/helix实现文件/前端掏空文件/wire要点/验收gate/可并行/依赖）+ worktree 并行性分析 |
| `03-测试策略.md` | 每个 HTTP 接口 + 每个 WS 事件 → 测试类/用例矩阵 + 覆盖维度 + 100% 覆盖率度量（cargo-llvm-cov）+ 每 Phase gate 测试判据 |
| `04-cses-client接入方案.md` | **最后一步、仅设计不实现**——二进制替换 + IPC 兼容层 + 灰度开关 + 前端纯渲染改造清单 + 回滚 + 联调验收 |

### 三审收口摘要（终稿已吸收）

- **rv-tauri-seam**：A1 HTTP 出站现网在浏览器 Angular `app-http.service.ts`+`networkInterceptor`（鉴权头/401/断网/traceparent 全在前端）→ 新增 **P1.5 出站横切层迁移**；A2 否决凭空发明的泛型 `im_outbound`，全改**语义命令**；A3 path 修正 `src/pages/message-v3`；A4 `imDataSource:*` 是 dead SQLite 垫片 → P6 复用 `store_get_messages` 语义并消除 3 个 early-return 拦截块。
- **rv-core-boundary**：C1 禁泛型命令（args 形态本身是业务知识）；C2 冻结投影 payload schema 真源（DialogItem/messages 全字段）做纯渲染证伪；C3 时间/ID 经 `Clock::now_ms`/`IdSource::next_id` 注入，gate 加 grep `SystemTime|Instant::now|Utc::now` 必空；C4 outbound 改 **linkme distributed_slice registry**（对齐 WS handler）解 P3 合并热点；C6 mp_ token 注入留 driver（grep `mp_` 在 core 必空）；C7 driver 批处理 `Vec<Effect>`（HX-C007）。
- **rv-correctness**：B1 **post/read 不是 bitmap**（`im_post_read` 活在 is_outbound + type=6 `apply_read_op` 活列），保留在 scope；B2 P3a/P3b「零交集」仅在 outbound registry 化后成立；B3/B6 send echo 锚 `action="post"`(非 posted) + `createAt`(非 createdAt) + cursor 锚 `nextSeq`(非 max event.seq) 列入 gate fail-hard 断言；B4 新增 **SyncScheduler 正交子机**（全局 sync 并发窗口，解 257 channel 重连风暴）；B5 unread/lastPost 走字段级 `BatchUpdate WHERE PK` 禁 read-modify-write。

### 遗漏接管面（终稿补入对应 Phase）

M1 重连 in-flight PendingSend 与 increment 双份去重（temporaryId 维度，P1/P4）；M2 文件上传留 driver 直通不进 core（P1 合理例外）；M3 connectionId hello 回注 → HTTP header 时序（P4）；makeTopic 双帧（已在 is_outbound，P3b 补字段）；多端 self echo 归属（connectionId 维度，P4）；userSnapshot 的 ucenter 跨 feature 读依赖（P1 阻塞项，§开放决策）。
