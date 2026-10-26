# Helix 每日代码 Review — 2026-06-24

> 自动生成（每日 review 任务）。被 review 范围：`c483cfb^..5ddc795`（2026-06-24 当天全部提交）。
> 当前 HEAD = `5ddc795`，分支 = `main`。统计：99 文件，+7824 / −2342。

---

## ① 昨日提交概览

当天共 **约 49 个提交**（zlc 与 qiaohui 两条并行线 + 多次 `merge origin/main` 整合），主题高度集中在三件事：

1. **大重构（占行数主体，纯搬迁）**
   - `module.rs` 1318 → 432 行：编排逻辑拆到 `lifecycle.rs` / `send_flow.rs` / `sync_flow.rs` / `query_dispatch.rs` / `port_reply.rs` / `correlation.rs`。
   - `acl/to_effect.rs` 861 → 24 行（降为 facade）：拆到 `transport_effects.rs` / `storage_effects.rs` / `sync_effects.rs` / `sync_http_effects.rs` / `projection_message_effects.rs` / `projection_control_effects.rs`。
   - 目的 = 收敛 `structure-gate.sh` 的 300 行硬顶债（白名单 baseline 全部只减不增）。

2. **outbound / sync 契约偏差修复（真正的功能改动，与现网 Go 对齐）**
   - 发送：`send_build.rs` 透传 `type`（DOCUMENT 富媒体不再降级 TEXT）+ `id`（编辑走 update 而非 create）—— UC-1.2 / UC-1.6。
   - 已读：`posts_existing.rs` 补 `posts[]` 列表模式 + `startTime` —— UC-3.2。
   - 退群：`channel_existing.rs` 透传 `userId` / `newCreator`（群主转让不再丢失）—— UC-5.3。
   - 模板回执：`template_received.rs` 入参键 `post_id` → camel `postId`（修死命令）—— UC-3.3。
   - HTTP method：`registry.rs` 新增 `method()`，已知 GET/DELETE 命令按现网映射；`X-Auth-Kind` 头改为 session/bot 都显式下标。
   - sync 一致性：`sync_increment.rs` allHash 改为 **byte-exact** `"<channelId>:<seq>"` `\n`-join 喂 FNV-1a（与现网 `all_hash.rs` 字节对齐，否则两端永久 hashMismatch → sync 风暴）。
   - 心跳补偿消费侧：新增 `pong_compensate.rs` + `ws/handlers/pong.rs` + `SyncScheduler::should_pong_compensate`（5s 节流）—— UC-4.4，补齐此前「只有产侧无消费侧」的缺口。
   - 在线 push 兜底：`ws/handlers/post.rs` 陌生 channel 由静默丢帧改为 cursor=0 惰性注册 —— UC-4.5。

3. **FFI 横切 + 生命周期 + 大量 e2e 脚本与测试**
   - FFI HTTP 走 `CrossCuttingHttp`（401/断网回报复用 event_sink）；`api.rs` 新增 `AuthTokenRegistry` 路由（Authorization→bot、cookieId→session）。
   - `pump.rs` drain 后显式 `runtime.shutdown_timeout(ZERO)`，根治 cpp-smoke 30s 孤儿 `getaddrinfo` race。
   - 新增 `api_smoke.rs` / `http_crosscut_composition.rs` / `pong_compensate_tests.rs` / `sync_scheduler_tests.rs` / `uc44`/`uc45` / `p1_send_build_test.rs` 等单测，及十余个 host-cli e2e 脚本。

整体质量评价：**当天工作以「重构收敛 + 与现网 wire 契约对齐」为主，注释密度极高、几乎每个改动都标注了现网 source-of-truth 行号与 UC 编号，并配套了可证伪单测**。这是一个偏「治理与正确性」而非「新增能力」的高质量日。

---

## ② 逐项优化建议（按严重程度排序）

### 🟠 中等

**M1. `ws/handlers/post.rs` 陌生 channel 自动注册 → channels map 无上限增长风险**
- 位置：`PostHandler::handle`，`ctx.state.channels.entry(ev.channel_id).or_insert_with(|| Channel::new(ev.channel_id, 0))`。
- 问题：改动把「未注册 channel 静默丢帧」改成「无条件惰性建 Channel」。语义上对齐了现网 `on_ws_event` 的 `entry().or_insert_with`，是正确的。但副作用是：**任何到达的 inbound post 都会在 `channels` 里建条目**。若服务端（或异常/恶意上游）推送大量不同 `channelId`，map 会单调膨胀，整个会话期不回收（只在 hello reset 清）。
- 建议：（a）确认 `channel_id` 在 parser 边界已强制为合法 26 字符 Id26（看注释应已校验，建议在报告里留证）；（b）评估是否需要对「会话内新建 channel 数」设一个软上限或在 `reset` 之外加 LRU/逐出策略；（c）至少补一条 EXPECT/不变量注释说明「上界 = 用户真实可见 channel 数，非攻击面」。当前判断为**可接受但需显式记账**，故列中等。

**M2. `posts_existing.rs` posts 列表「全有效才注入」是静默降级**
- 位置：`PostReadCommand::build`，`if !posts.is_empty() && posts.len() == arr.len()`。
- 问题：只要数组里有**一个**非字符串元素，整个 `posts` 字段被丢弃，回退到区间模式。这是「坏一个丢全部」的静默行为，前端传了脏数据时不会有任何反馈，已读范围可能错。
- 建议：要么收集所有合法 string（`filter_map` 后非空即注入，容忍个别坏元素），要么在 `posts.len() != arr.len()` 时返回 `ImError::Parse` 显式拒绝。当前实现两头不靠。

### 🟡 低

**L1. 投影 emit 工厂大量 `.expect("...static JSON...")`（egress 序列化）**
- 位置：`acl/projection_message_effects.rs` / `projection_control_effects.rs` / `sync_effects.rs` 等，约 15 处 `serde_json::to_vec(&payload).expect(...)`。
- 说明：这些是 to_effect.rs 拆分时**搬迁过来的既有代码**，不是当天新引入的缺陷；且对象是代码自建的 `json!` 值（非边界输入），实际不会 panic，`.expect` 文案也写明了不变量。不违反「边界输入零 unwrap」铁律。
- 建议（可选，长期）：egress 路径若想做到「字面零 panic」，可统一改 `unwrap_or_default()`（坏 payload 发空 body 而非崩进程）。优先级低，纯防御性。

**L2. `outbound/registry.rs` method 映射用「命令名长 match」**
- 位置：`outbound_method(name: &str)` 的 `match name { "im_team_quit" | ... => "DELETE", ...大段 GET列表... }`。
- 说明：当前 O(1) 字符串 match 没有性能问题，但与本仓「Strategy/registry 去巨型 match」的设计取向略有张力——GET/DELETE 名单会随命令增多继续变长，且与 `OutboundCommand` trait 的就近声明分离（改一个命令的 method 要回到这张表）。
- 建议：考虑把 `method()` 下沉为各 `OutboundCommand` 实现的 override（GET/DELETE 命令各自 `fn method()` 返回真值，默认 POST），与 `is_read()` / `auth_kind()` 的现有模式一致，删掉中心表。非紧急。

**L3. `should_pong_compensate` / `pong_throttle` 的 map 同样长期累积**
- 位置：`sync_scheduler.rs`，`pong_throttle: HashMap<ChannelId, u64>`。
- 说明：每个被补偿过的 channel 留一条时间戳，仅 `reset`（hello）清。上界 = 会话内 channel 数，与 M1 同源、量级可控，仅记录。

---

## ③ 架构对齐结论

**结论：无内核架构漂移（drift）。当天的「漂移信号」全部落在治理护栏层，且已在当天回正收敛。**

逐项核验：

- **HX-C001（core 零运行时/零平台依赖）✅**：`grep -rnE 'use (tokio|sqlx|reqwest|web_sys|tauri)' crates/helix-core/src` 为空。功能改动全部落在 helix-im（业务模块层）与 driver 层，core 未被污染。
- **HX-C002（Send 边界 / 状态禁 `&'a`）✅**：`pong_compensate` / `sync_scheduler` 新状态用 owned `ChannelId`(Copy) + `HashMap`，未引入生命周期借用进 async/闭包。
- **HX-C005（热路径 O(1)）✅**：allHash 改写仍是一次性 join + 单遍 FNV；pong 节流是 `HashMap` test-and-set O(1)；未见 read-modify-write。
- **HX-C006（FFI 边界）✅ 且强化**：`pump.rs` 的 `shutdown_timeout(ZERO)` 正是「生命周期不被网络 I/O 阻塞」红线补到 runtime 收尾这一环，方向与 engine 五不变量一致。
- **HX-C008（sync cursor 锚）✅**：post.rs 陌生 channel seed=0（本地未确认水位，非误种服务端水位），缺口由 gate 1s 回拉兜底，符合单调不回退契约。
- **HX-C010（确定性有序）✅**：`compensate_from_pong` 对 hashMismatch 全量根群 `sort_unstable` 后再产 Effect；allHash 排序后拼串。两处都显式防 SipHash 随机序。
- **HX-C011（测试可证伪）✅**：`all_hash_is_string_form_not_raw_bytes` / `fnv1a_canonical_anchor_vector` 是「实现回退即 FAIL」的破坏性锚点，非墙钟下界断言。
- **代码结构（≤300 行硬顶 + baseline 只减不增）✅**：`bash scripts/structure-gate.sh` 当前 **exit 0**。白名单 baseline 全部往小改（module.rs 1318→432、to_effect/state 直接移出白名单、storage 578→560），未出现「上调 baseline 抹 FAIL」的棘轮后门。新拆出的文件除 `module.rs`(432,在白名单)外均 ≤300（lifecycle 291 / port_reply 251 / posts_read 216 …）。
- **GOAL §1/§2（sans-IO / 确定性 / 三层单向）✅**：pong 补偿是纯算 Effect（HTTP 由 driver 发）、`now_ms` host 注入、不读墙钟，符合 sans-IO。

**佐证**：项目自身的 `architecture-drift-review`（第十四轮，区间 `7b6ea55..157f315`/`..413c4a8`）已对当天主体批次跑过，结论为「🔴 重 drift 但 100% 在治理护栏层（structure-gate baseline 后门第 3 次复发 + gate.sh 未接 structure-gate），内核 HX-C001..C015 / GOAL §1·§2 全 HOLD」，并已回正（`d0d2936` 四文件拆分 + gate.sh 接入 structure-gate；`c81de35` state.rs 外提收敛）。本日 review 独立复核与该结论一致。

**唯一需提示的流程项（非架构漂移）**：
- `workflow-review-ledger.md` 的 `last_reviewed_commit` 当前停在 `6679e40`，而 HEAD 已到 `5ddc795`，`gap = git rev-list --count 6679e40..HEAD = 13 ≥ 10`。按 `.claude/rules/agent-review-cadence.md`，**下一个 commit 检查点应触发一次 `architecture-drift-review`**（这 13 个增量主要是 codex 的 to_effect/module 二次拆分 + e2e 脚本 + 收尾 merge，预计仍是治理/测试层，轻量）。这不是当天改动的缺陷，而是闸门台账待推进。

---

## ④ 总体评价与下一步

**总体评价：优秀的一天。** 改动以「与现网 Go wire 契约逐字对齐 + 结构债收敛」为主线，每处偏差修复都标注了现网 source-of-truth 与 UC 编号、配了可证伪单测，重构把两个最大的超限文件（module.rs / to_effect.rs）成功拆解并把结构闸 baseline 整体往下锁。内核架构不变量全部保持，未见真正的架构漂移。bug 风险面集中在两处「静默行为」（陌生 channel 无限注册、posts 列表坏一个丢全部），均为可控的边界健壮性问题而非正确性破裂。

**下一步建议（按优先级）：**

1. **推进 drift-review 台账**：gap=13≥10，在下个 commit 检查点跑一次 `architecture-drift-review` 并把 `last_reviewed_commit` 更新到 `5ddc795`，让闸门归零。
2. **M1 显式记账**：给 post.rs 陌生 channel 自动注册补一条「上界 = 真实可见 channel 数」的不变量注释，或评估软上限/逐出，消除潜在的 map 无限增长担忧。
3. **M2 去静默降级**：`posts_existing.rs` 的 posts 列表改为「容忍个别坏元素收集合法项」或「不一致即 Parse 报错」，二选一。
4. **L2 可选收口**：把 `outbound_method` 中心表下沉为各命令 `method()` override，与 `is_read`/`auth_kind` 模式统一。
5. **验证补强（建议跑一次）**：本 review 未执行 `cargo test`（遵守仓内 Cargo 纪律，避免编译空耗）。建议在合适检查点跑 `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi && cargo test -p helix-core` 与 `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'`（应空），把「绿」作为收口凭据（项目自报当天 `cargo test 563/0`）。

---
*报告由每日 review 任务自动生成；如需对某个提交做更细的逐行走查，可指定 commit hash。*
