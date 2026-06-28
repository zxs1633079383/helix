# Helix 每日代码 Review — 2026-06-23

> 自动生成（每日 code review 定时任务）· 生成时间 2026-06-24
> Review 范围：`7d8d91b8^..e98adf57`（昨日 35 个提交，作者 zlc）
> 改动规模：114 文件 +7580 / −1121；其中 `.rs` 64 文件 +4596 / −996

---

## ① 昨日提交概览

昨日共 **35 个提交**，是一个高强度的「cses 全量纯 helix 化接管」推进日。按主题归类：

| 主题 | 提交数 | 代表 |
|---|---|---|
| **outbound 命令扩充** | 9 | vote/score 第二网关 10 命令、置顶 messageTop/Untop、公告 save/delete/read、channel permission、createPosts 批量转发(88→89)、bot_agent 读命令标 is_read |
| **新功能（投影/回灌）** | 4 | `im_load_older_context` 多轮翻页(D3 上拉)、`im_query_dialog_list` 会话列表投影、spec06-FixA 读族 HTTP 响应回灌、spec06-FixB message-row fat emit 完整投影 |
| **F1 发消息根治** | 2 | HTTP 200 兜底对账杀 self-omit echo 永 spinner、im_reconnect 控制臂 |
| **性能（E9b）** | 1 | batch_upsert SQL 按形状缓存去 per-call 分配 |
| **治理/结构** | 3 | 十三轮 drift-review 收敛(state.rs 测试外提)、structure-gate 测试外提回正、fullmap-coverage 计数 bug 修 |
| **工具链/CI/harness** | 5 | harbor 镜像、HX-C014/C015 卡、夜间 loop 工具链、7 个具名 workflow |
| **文档/spec/session** | 11 | 全量纯 helix 化设计 spec、迁移图谱、curl 真源、SESSION/ROADMAP 同步 |

整体方向：把现网 cses（mattermost csesapi 后端 + cses-client 前端）的 IM 能力**全量替换**到 helix 引擎，前端退化为纯渲染层。昨日把 outbound 命令补到 89，并补齐三类「读回灌通道」（读族 HTTP 透传、dialogList 投影、上拉翻页）。

---

## ② 逐项优化建议（按严重程度排序）

### 🔴 高 — 无

未发现会导致数据损坏、panic、双端碎裂或安全问题的高危项。新增代码普遍遵循「边界零信任 + Result + 不 panic」纪律。

### 🟡 中

**M1. `older_context.rs`：空 `temporaryId` 的 wire Post 被静默丢弃**
`older_context.rs:143-152`，`ingest_round` 仅当 `temporaryId` 非空且 `seen.insert` 成功才把行 push 进 `older`：

```rust
if !tmp.is_empty() && self.seen.insert(tmp) {
    self.older.push(row.clone());
}
```

去重键 = `temporaryId`。若服务端某条 Post 的 `temporaryId` 为空（理论上自己发的消息回程可能缺该字段），该条会被丢出上拉结果，用户上拉看不到这条历史。这与 legacy 行为是否一致需确认——legacy 用什么键去重？建议：① 在 doc 注明「假设服务端 postContext 行必带非空 temporaryId」并补一条断言/日志；② 或退化用 `id`（server id）作为后备去重键（`id` 在 pivot 选择处已证明服务端行带非空 id）。当前为**潜在数据缺失**，非崩溃，故定中。

**M2. `query.rs build_dialog_list_query`：整表 scan + `ORDER BY last_post_at DESC` 缺索引保证**
`query.rs:120-130`，dialogList 投影对 `channel` 表全量 scan、按 `last_post_at` 降序、limit 最多 2000。doc 已声明「低频 off-hot-path 合规」，这点没问题；但若 `channel.last_post_at` 无索引，2000 行的排序仍是全表 sort。建议：确认 schema 对 `last_post_at` 建了索引（或在 projection-schema/migration 注明），否则会话量大时单次拉取延迟会随会话数增长。属可维护性/性能护栏，非 bug。

### 🟢 低 / 命名

**L1. `unwrap_sync_envelope` 名称已名实不符。**
`module.rs` 现在对**读族回灌**(`OutboundReadReply`)和**上拉翻页**(`LoadOlderContext`)的 HTTP PortReply 也调 `unwrap_sync_envelope`。经核对，driver 端 `run_http_envelope`（`pools.rs:71`）对**所有** `Effect::Http` 响应统一编码 ADR-007 信封（status/headers/body:base64），所以**功能完全正确、无集成 bug**。但该 helper 名字带 `sync` 会让后来读者误以为只适用于 sync/notify 路径。建议改名 `unwrap_http_envelope`（纯重命名，零行为变更）。

**L2. `read_relay` / `older_context` 的「透传」实际是 reparse→reserialize。**
`read_relay.rs:22` 和 `older_context.rs:280` 把后端 body 先 `from_slice` 再嵌入再 `to_vec`。对读族/上拉（单次≤100 行）量级完全可接受，且这是 passthrough 嵌 JSON 的必需代价。仅提示：不要把这套模式复制到每消息热路径（会违 HX-C005）。当前用法合规。

**L3. `.expect("... static JSON shape must serialize")` 用在含动态数据的 payload 上。**
`older_context.rs:291`、`query.rs:177` 等处对包含服务端 `messages`/`rows`（动态 Value）的 payload 用 `.expect`。`serde_json::to_vec` 对已解析的 `Value` 实际不会失败（NaN/Inf 的 f64 会被序列化为 `null` 而非报错），所以**不会 panic**；但注释「static JSON shape」对含动态字段的 payload 略有误导。可改注释为「Value 已就绪，序列化不会失败」。极低优先级。

### 测试覆盖：✅ 良好

新增逻辑均带对应测试，且遵循项目的「测试外提 sibling」结构纪律：
- `older_context_tests.rs`(83) + `p3c_load_older_context_test.rs`(248) — 上拉翻页续/停决策、信封剥离、空批收尾
- `read_relay_test.rs`(250) — 读族回灌成功/失败/信封畸形三路 + corr 注册闸
- `query_tests.rs`(206) + dialogList 投影
- `vote_score_tests.rs`(221) — 10 命令 body/读写族/网关
- `registry_tests.rs`(192)、`upsert_sql.rs` 内置缓存命中测试、`f1_gap1_reconnect_read_channel_test.rs`(162)、`cp_create_posts_test.rs`(206)

`projection_schema_test.rs` 同步更新(+213)，对投影字段集强制对账。提交体声明 `cargo test -p helix-im` 全量 0 failed（本 review 未重跑 cargo，遵循项目 Cargo 纪律避免分钟级编译空耗）。

---

## ③ 架构对齐结论

**结论：🟢 无架构漂移（drift）。昨日改动高度贴合既定架构与不变量。**

逐条核验关键 harness 不变量与 GOAL：

- **HX-C001（core 零运行时 / 零 host 硬编码）✅**：新增 `Gateway::{Im,Default}` 是「意图维度」枚举——core 只标命令走哪个网关，真实 base host 由 host 经 `ImConfig.default_api_base_url` / `HostNetworkConfig.default_api_base_url` 注入。与既有 `AuthKind` 同构，core/im 不出现任何 host 字面量。`older_context`/`read_relay` 纯产数据零 I/O。
- **HX-C002（owned 状态 / 禁 `&'a` / Send 边界）✅**：`LoadOlderState` 全 owned 字段，进 `corr_map` 跨 PortReply 存活；`CorrelationContext::LoadOlderContext` 用 `Box<LoadOlderState>` 装箱避免撑大枚举；`OutboundReadReply{req_id: String}` owned。为容纳 `serde_json::Value`(f64 非 Eq) 主动去掉 `CorrelationContext` 的 `Eq` derive，仅留 `PartialEq`，并注释了理由（grep 实证 Eq 无用）——处理得当。
- **HX-C005（热路径 O(1) / 零分配）✅**：E9b 把 batch_upsert 的 SQL 文本按形状缓存（thread-local `HashMap<UpsertShapeKey,String>`），消除 per-call `format!`/`join`/`Vec` 分配，行为逐字等价、带命中测试。dialogList/上拉为**显式低频** off-hot-path，doc 注明。
- **HX-C001 sans-IO 编排 ✅**：上拉「legacy 8 轮 for 循环」被正确改写为 **corr_map 续接状态机**——每轮 postContext 经 `Effect::Http` → PortReply 回报 → `ingest_round` 决定续(alloc 新 corr 重发+重注册)/停(emit)，绝不在 core 同步 loop。`MAX_ROUNDS=8` 硬上限防服务端病态返回打满 HTTP。
- **HX-C010（确定性有序 Effect）✅**：`emit_older_loaded` 把 emit 设为**唯一权威排序点**（createAt 升序 + 同毫秒 temporaryId 确定化），不依赖中途 `stop()` 是否已排，对已排序输入幂等。
- **边界零信任（GOAL §2 / 不变量4）✅**：所有新解析入口（`parse_request`/`extract_post_rows`/`read_req_id`/`build_dialog_list_query`）缺字段/坏 JSON 一律 `Err` 或安全降级，零 `unwrap`/`expect` 于外部输入；HTTP 失败/信封畸形都回灌 `{req_id, error}` 让前端 reject 而非永久挂起 spinner。
- **结构纪律（code-structure-ddd §1）✅**：所有新/改 `src/**.rs` 均 ≤300 行或收口到 baseline。治理提交把 `vote_score.rs`(446→231)、`state.rs`(366→351 baseline)、`module.rs`(1319→1318) 全部拆回，**且未上调任何 baseline**（严守「只减不增」红线，避开 drift-review 反复点名的 baseline 后门）。`network.rs` 把 URL/header 工具裂分到 `network_util.rs` 守 300 行。
- **治理闭环自洽 ✅**：`fullmap-coverage.sh` 修了计数 bug——旧正则不含 `/api/cses` 前缀导致 HTTP 假报 0/84、WS 硬编码漏算报 15/19；修后回到**真实** HTTP 39/79、WS 18/18。这是把「虚假绿」纠成「诚实数字」的正向治理（与 HX-C009/C012「可证伪、不自产自判」精神一致）。昨日同时落地 HX-C014/C015 与多轮 architecture-drift-review 收敛提交，说明项目自身的 review cadence 在正常运转。

唯一**观察项**（非 drift）：`fullmap-coverage` 修复后暴露 HTTP 端点真实覆盖率仅 **39/79**。这与多个提交里「100% 生产可用·全量切换」的措辞存在**表述 vs 现状的落差**——属正常迁移中途状态，但建议 SESSION/ROADMAP 用真实覆盖数（39/79、18/18）描述进度，避免「100%」字样与闸门实测数字打架，给后续 review 制造噪声。

---

## ④ 总体评价与下一步建议

**总体评价：高质量推进日。** 昨日 35 提交在大体量改动下仍保持了少见的工程纪律：意图维度抽象（Gateway/AuthKind 同构）、sans-IO 状态机改写（上拉翻页）、owned 状态、确定性排序、测试外提、baseline 只减不增、以及把虚假覆盖率纠成诚实数字。新增逻辑测试覆盖完整，未发现高危 bug 或架构漂移。代码注释质量很高（讲清「为什么」与不变量），符合 §5 风格要求。

**下一步建议（按优先级）：**

1. **（中）确认上拉翻页去重键的健壮性**：核对 legacy `fetch_older_posts_by_context` 的去重键，决定 `older_context.rs` 是否需要 `id` 后备键，避免空 `temporaryId` 行漏显（M1）。
2. **（中）补 `channel.last_post_at` 索引保证**：在 migration/projection-schema 确认或补建索引，护住 dialogList 投影的 scan+sort（M2）。
3. **（低）重命名 `unwrap_sync_envelope` → `unwrap_http_envelope`**：消除名实不符，纯重命名（L1）。
4. **（低）进度表述对齐实测覆盖率**：把 SESSION/ROADMAP 的「100%」改为闸门真实数字（HTTP 39/79、WS 18/18），并把 HTTP 覆盖缺口（40 个未接管端点）列入 ROADMAP 活账。
5. **（流程）保持 drift-review cadence**：昨日改动量大（35 commit），按 `agent-review-cadence` 的 gap≥10 阈值，应已触发或临近 `architecture-drift-review`；建议在下个 commit 检查点跑 `scripts/review-gap.sh` 确认 ledger `last_reviewed_commit` 已更新到本批 HEAD。
