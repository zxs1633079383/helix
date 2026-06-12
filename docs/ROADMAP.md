# Helix 细粒度 Roadmap · TodoList · 验收 Checklist

> 2026-06-12 · 基于：技术过会方案 §6（M0-M4）+ 评审 D 系调整（+3 周）+ 实际进度对账。
> 当前位置：**按日历 M2 第一周，按交付物 M2 中后段**（M1 全部 + M2 约 60% 已完，见 git log `519bbde..85e07ec`）。

---

## 一、细粒度 Roadmap（天级）

### M2 收官 · 第 1 周——真连 Go 服务端（L2 联调）

| 天 | 交付 | 验收 |
|---|---|---|
| D1-2 | **最小 native 宿主**（`crates/helix-host-cli`）：engine_loop + ImModule + 真实 config 装配；连 dev 环境 Go IM 完成 hello 握手 | 握手日志 + `im:connection:established` 事件可见 |
| D2-3 | **UC-1 端到端 smoke**：发消息 → echo 对账 → 查库验证；录制真实帧 → golden fixtures（im_trace 导出 → Tick 剧本转换小工具） | smoke 绿 + fixtures 入库 `tests/fixtures/` |
| D3-4 | **B4** event_seq 三级 fallback 解析 + **B5** phantom 事件（联调必撞的两个）；**B2** increment_end 冷启动自愈 | 各配 replay 测试 |
| D5 | **E5-E7** 三个 P1（DDL 出热路径 / gate buffer 上限+主动 too_long / broadcast lagged 可感知）+ 重跑压测对照 | stress_storage cursor 路径 ≥ 30K/s |

### M2 收官 · 第 2 周——Tauri 换引擎（四接缝）

| 天 | 交付 | 验收 |
|---|---|---|
| D1-3 | 接缝①②：10 个生产命令内部切 Tick / 读旁路；`NativeEventSink` → `app.emit('im:__bus__')`；**灰度开关** | cses-client 前端零改动跑通收发 |
| D3-4 | 接缝③④：同库同表灰度共存；**B7** store:\*/imDataSource SQL 语义逐字段对齐；**B3** readBits 单调覆盖 | 开关切换 + 秒级回退演练 |
| D5 | M2 总验收（见 Checklist §三）+ 文档对账同步 | 全 gate 绿 → 申请 M3 |

### AI 工程化补课（0.5 周，可与上面并行——见 §四差距）

CI 三栖 matrix → clippy deny-list → per-crate CLAUDE.md → cargo public-api gate（优先级从左到右）。

### M3 —— wasm 浏览器端（3-4 周，按 D2 调整）

1. W1：**IndexedDB 兜底**先跑通 Tick-Effect 循环（D2 拍板：OPFS 后置）；wasm driver 从空壳到四能力可用
2. W2：Worker 接入 + postMessage 批量桥 + 读查询 request_id 路由（§4.4.2）
3. W3：OPFS（SAHPool + COOP/COEP）升级 + 浏览器 IM demo + wasm 包体积（opt-level=z 链路 + twiggy）
4. W4：缓冲 / 三端回放一致性测试（同一剧本 native vs wasm 输出比对）

### M4 —— FFI 移动端（2-3 周，按 D5 调整）

1. uniffi 对 `Vec<Effect>`/嵌套 enum 实测（D5：4→7 天）；spawn_blocking 落地（MAJ-4/HX-C006）
2. helix_command/helix_query C ABI + SendPort 事件流 + Flutter demo
3. panic=abort cdylib profile + 双端冒烟

---

## 二、TodoList（活账，按优先级）

**P0（M2 收官阻塞）**
- [ ] 最小 native 宿主 bin + Go 服务端 UC-1 联调
- [ ] B4 event_seq 三级 fallback · B5 phantom · B2 increment_end 自愈
- [ ] Tauri 四接缝接入 + 灰度开关（含 B7 store:\* 对齐、B3 readBits）

**P1（质量与性能）**
- [ ] E5 monotonic_upsert DDL 出热路径 · E6 gate buffer 上限+主动 too_long · E7 broadcast lagged 可感知
- [ ] B6 UpsertSpec conflict 列语义文档
- [ ] golden fixtures 工具链（im_trace 导出 → Tick 剧本）

**P2（性能尾巴 + 工程化）**
- [ ] E8 timer 反向索引 · E9 热路径分配收口（HX-C005）
- [ ] AI 工程化四件：CI matrix / clippy deny / per-crate CLAUDE / public-api gate（§四）
- [ ] 四能力 feature 拆分（§7 技术债：driver-native optional dep）
- [ ] bench 常态化（criterion 化 stress 三件套，数字进 CI 留痕）

**已销账**：A 全部 · B1 · C1-C4 · E1-E4 · M1 全部 · M2 的 driver/im 主体。

---

## 三、最终验收 Checklist（全部 machine-checkable，过一项勾一项）

**结构闸门（每次 merge 必跑）**
- [ ] `cargo check` 三栖（native 4 crate + wasm 2 crate）= 0 error
- [ ] `cargo test --release` workspace 全绿（当前基线 85，只增不减）
- [ ] `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` = 空
- [ ] `grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ | grep -v '//'` = 0
- [ ] harness 索引不变量：CLAUDE.md 表行数 == `ls docs/harness/HX-C*.md | wc -l`
- [ ] （落地后）`cargo public-api -p helix-core` diff 对 allowlist = 0

**行为闸门（承压基线，压测三件套）**
- [ ] proptest 乱序不变量 5 条绿（cursor 单调 / 不丢消息 / 确定性）
- [ ] 10k 裸灌不死锁 + graceful drain 后查库全落盘
- [ ] batch_upsert 5000 行 < 100ms（写放大 ~1.0x）

**M2 联调闸门（真连 Go）**
- [ ] UC-1：发消息 → 0 等待上屏 → echo 对账「已送达」→ 库中 id 已换正式 id
- [ ] UC-2：断 WS 30s → 重连 → deadQueue 续联或 sync 补齐，消息无洞
- [ ] UC-3：模拟离线 >500 事件 → 自动续拉至 no_change，cursor == 服务端 max seq
- [ ] UC-4：多 channel 批量 sync，置顶优先
- [ ] Tauri 灰度：Angular `git diff` = 0；开关回退 < 5s；同一 DB 双引擎数据互认

**M3/M4 各自追加**
- [ ] 同一 Tick 剧本三端输出 byte-level 一致（可回放跨端验证）
- [ ] wasm demo 浏览器收发 + 包体积 < 2MB；FFI Flutter 冒烟 + abort 语义验证

---

## 四、AI 工程化差距（对照 Bun Rust 重写仓，`/workspace/ai-workspace/bun`）

> Bun 的设计哲学：**把工程约束嵌入工具链（clippy/hooks/CI），让 AI 的错误选择触发自动反馈，而不是等 review**。

| 层 | Bun 机制 | Helix 现状 | 差距动作（按 ROI 排序） |
|---|---|---|---|
| 护栏 | **clippy.toml 29 条 deny-list**（std::fs→bun_sys 等，编译期 API 边界） | HX-C001/C005 靠 grep + review | **①** `clippy.toml`：helix-core 内 deny `std::time::Instant::now`/`SystemTime`（必须走 Clock port）、deny `println!`；workspace deny `unwrap` in 边界路径——把 harness 卡编译期化 |
| 验证 | CI 多 job 闸门链 + 修改必附测试 | 全部 gate 靠手跑/会话纪律 | **②** `.github/workflows/ci.yml`：三栖 matrix + cargo tree gate + grep gate + test + （后续 public-api）——本文件 §三结构闸门全部自动化 |
| 指令 | **嵌套 CLAUDE/AGENTS.md ×11**（就近继承）+ settings.json hooks 自愈 | 仅 root CLAUDE.md + 1 个 rules 文件 | **③** 每 crate 一份薄 CLAUDE.md（core：零依赖铁律+不变量；im：剧本测试 pattern+ACL 约定；driver：worker/背压契约+压测命令）；PostToolUse hook 自动 rustfmt |
| 流程 | **.claude/skills 7 个 + commands 6 个**（pattern 即代码） | docs/harness 7 卡（雏形同构） | **④** harness 卡毕业为 `.claude/skills/`（写新模块 / 写回放测试 / 跑承压验收 三个 skill 起步） |
| 锁定 | rust-toolchain.toml + rustfmt.toml pinning | 无 | **⑤** 两个文件各 5 行，一次性 |
| 参照 | **Zig-as-reference**（.zig sibling 是行为真源） | 已在实践（现网 cses-client 逐文件核验）但未成文 | **⑥** 写进 root CLAUDE.md 一句：「行为争议时 `cses-client/src-tauri/features/im` 对应 handler 是 source of truth」 |
| 验证 | bench 42 域 + 无数字性能主张自动拒 | 压测三件套已入库但非常态 | **⑦** criterion 化 +「性能 PR 必附 before/after」写入规则 |

**已对齐不用补的**：结构化 commit + 模块化 tag（≈ Bun 的 PR 纪律）、对抗 review 流程（≈ Landing PR checklist 的 agent 化）、回放测试基建（≈ harness.ts 思想）、待办清单销账制（≈ issue 流程的轻量版）。

**Bun 给 5-crate 小项目的三条核心移植**（Explore 结论）：分层指令而非集中、clippy deny 当 API 契约、skills/commands 当 AI 课程——对应上表 ③①④。
