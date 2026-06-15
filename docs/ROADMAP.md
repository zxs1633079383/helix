# Helix 细粒度 Roadmap · TodoList · 验收 Checklist

> 2026-06-12 · 基于：技术过会方案 §6（M0-M4）+ 评审 D 系调整（+3 周）+ 实际进度对账。
> 2026-06-15 更新：按 ADR-006，网页端/wasm 暂停维护；产品化路线只保留 PC Tauri + Flutter 手机端。
> 当前位置：**按日历 M2 第一周，按交付物 M2 中后段**（M1 全部 + M2 约 60% 已完，见 git log `519bbde..85e07ec`）。

---

## 一、细粒度 Roadmap（天级）

### M2 收官 · 第 1 周——真连 Go 服务端（L2 联调）

> ⚠️ **2026-06-12 四专家 panel-review 重排**：原 D2-3 smoke 排在 B4/B5 之前 = 顺序倒置；且 UC-1 happy-path 跨的 3 块代码当时**死/缺/脆**（echo 死代码、parser 无 event_seq、`im:connection:established` 全仓 0 命中）。决策：**先补 4 块前置缺口再跑真 Go smoke**（用户拍板）。

| 天 | 交付 | 验收 |
|---|---|---|
| D1-2 | **最小 native 宿主**（`crates/helix-host-cli`）+ **UC-1 前置 4 块**（panel-review 揪出 happy-path 死/缺/脆）：① **hello 握手时序**——`Tick::Connected` 后先发 hello 帧、收 hello-ack 才置 Connected 并 **emit `im:connection:established`**（当前全仓 0 此事件，验收条件本不满足；resync 须等握手后发，否则 Go 401/403）② **B4** event_seq 三级 fallback **+ hello-ack 帧解析**（parser 当前无 event_seq、hello-ack 无 seq 被静默丢）③ **B5** phantom ④ **echo→reconcile 路由**（`handle_echo` module.rs:679 当前 `#[allow(dead_code)]` 从不被 Inbound 调用，tmp_id→正式 id 不覆写）| 握手日志 + `im:connection:established` 可见 + 4 块各配 replay 测试 |
| D3-4 | **UC-1 端到端真 Go smoke**：发消息 → 0 等待上屏 → echo 对账「已送达」→ 查库 temporaryId 已换正式 id；录制真实帧 → golden fixtures（im_trace 导出 → Tick 剧本转换小工具，注意时间/id 注入确定性）| smoke 绿 + fixtures 入库 `tests/fixtures/` |
| D5 | ~~E10~~（已落地 `d58dff1`）+ **B2** increment_end 冷启动自愈 + **多模块并发写 benchmark**（对抗审揪出的真缺口）+ 生产 Linux 真盘数字 | B2 配 replay；多模块并发写实测（单写者池竞争）；详见承压评估 2026-06-12 节 |

### M2 收官 · 第 2 周——Tauri 换引擎（四接缝）

| 天 | 交付 | 验收 |
|---|---|---|
| D1-3 | 接缝①②：10 个生产命令内部切 Tick / 读旁路；`NativeEventSink` → `app.emit('im:__bus__')`；**灰度开关** | cses-client 前端零改动跑通收发 |
| D3-4 | 接缝③④：同库同表灰度共存；**B7** store:\*/imDataSource SQL 语义逐字段对齐；**B3** readBits 单调覆盖 | 开关切换 + 秒级回退演练 |
| D5 | M2 总验收（见 Checklist §三）+ 文档对账同步 | 全 gate 绿 → 申请 M3 |

### AI 工程化补课（0.5 周，可与上面并行——见 §四差距）

CI 双端 matrix（native + FFI）→ clippy deny-list → per-crate CLAUDE.md → cargo public-api gate（优先级从左到右）。wasm CI 不再作为必跑项，除非未来 ADR 重新激活网页端。

### M3 —— Flutter 手机端 / FFI（2-3 周，按 ADR-006 调整）

1. F1：确定 Flutter FFI API 最小面：`open/start/send_message/on_event/destroy`，Dart 不碰 IM SQL / sync 细节
2. F2：`helix-driver-ffi` 复用 `helix-driver-host` 的 rusqlite/SQLite 存储，`StorageOp` 语义与 native 单一真源对齐（已落 shared conformance 测试）
3. F3：事件回 Dart Stream；panic=abort cdylib/staticlib profile；Flutter demo 冒烟
4. F4：同一 Tick 剧本 native vs FFI 输出一致；同一 schema/migration + Storage conformance 测试

### M4 —— 归档的 wasm/web（暂停维护）

`crates/helix-driver-wasm` 与 `docs/03-driver-wasm.md` 只作为历史实验保留。当前不做网页端、不补 IndexedDB/OPFS、不把 wasm 纳入交付验收。未来若重启网页端，先新增 ADR 重新定义 storage/cache 策略和 CI。

---

## 二、TodoList（活账，按优先级）

**P0（M2 收官阻塞）**
- [ ] 最小 native 宿主 bin（`crates/helix-host-cli`）——transport 路由表从一开始做成 `HashMap<TransportId, NativeTransport>`（非单连接）+ host 集中分配 TransportId，给第二模块/多端口（如 3399）留对接缝。设计见 `docs/02-im-module-mount-and-core-generality.md §4.4`
- [ ] **UC-1 前置 4 块**（panel-review 2026-06-12 揪出，smoke 必须排在其后）：
  - [ ] ① hello 握手时序：`Tick::Connected`→发 hello→收 hello-ack→置 Connected + emit `im:connection:established`（transport.rs:103-156 / module.rs:771-781 / 828）
  - [ ] ② B4 event_seq 三级 fallback + hello-ack 帧解析（parser.rs:29-56 当前无）
  - [ ] ③ B5 phantom 事件
  - [ ] ④ echo→reconcile 路由：Inbound 分支识别 echo 帧调 `handle_echo`（module.rs:679 当前死代码 / 715-738 Inbound 分支）
- [ ] B2 increment_end 冷启动自愈
- [ ] UC-1 端到端真 Go smoke + golden fixtures（排在前置 4 块之后）
- [ ] Tauri 四接缝接入 + 灰度开关（含 B7 store:\* 对齐、B3 readBits）

**P1（质量与性能）**
- [x] E5 monotonic_upsert DDL 出热路径（建表挪到 open()，monotonic_upsert 仅剩单 INSERT；回归锚 test_monotonic_table_created_at_open_e5）
- [x] E6 gate buffer 上限+主动 too_long（MAX_GATE_BUFFER=16384，超限清 buffer + emit too_long + sync 重拉，cursor 不回退；回归锚 ×2）
- [x] E7 broadcast lagged 可感知（接收侧 EventReceiver/RecvOutcome::Lagged 显式上报；纠正承压评估 emit 侧定位错误——send 侧感知不到 lagged；回归锚 ×2）
- [ ] B6 UpsertSpec conflict 列语义文档
- [ ] golden fixtures 工具链（im_trace 导出 → Tick 剧本）

**P2（性能尾巴 + 工程化）**
- [ ] E8 timer 反向索引 · E9 热路径分配收口（HX-C005）
- [ ] AI 工程化四件：CI 双端 matrix / clippy deny / per-crate CLAUDE / public-api gate（§四）
- [ ] 四能力 feature 拆分（§7 技术债：driver-native optional dep）
- [ ] bench 常态化（criterion 化 stress 三件套，数字进 CI 留痕）

**已销账**：A 全部 · B1 · C1-C4 · E1-E4 · **E5-E7** · M1 全部 · M2 的 driver/im 主体。

---

## 三、最终验收 Checklist（全部 machine-checkable，过一项勾一项）

**结构闸门（每次 merge 必跑）**
- [ ] `cargo check` 双端产品目标（core/im/driver-native/driver-ffi）= 0 error
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

**M3 追加（Flutter FFI）**
- [ ] 同一 Tick 剧本 native vs FFI 输出 byte-level 一致（可回放跨端验证）
- [ ] FFI Flutter 冒烟 + abort 语义验证
- [ ] `helix-driver-host` storage conformance 测试覆盖 native/ffi 共享 SQLite 语义

---

## 四、AI 工程化差距（对照 Bun Rust 重写仓，`/workspace/ai-workspace/bun`）

> Bun 的设计哲学：**把工程约束嵌入工具链（clippy/hooks/CI），让 AI 的错误选择触发自动反馈，而不是等 review**。

| 层 | Bun 机制 | Helix 现状 | 差距动作（按 ROI 排序） |
|---|---|---|---|
| 护栏 | **clippy.toml 29 条 deny-list**（std::fs→bun_sys 等，编译期 API 边界） | HX-C001/C005 靠 grep + review | **①** `clippy.toml`：helix-core 内 deny `std::time::Instant::now`/`SystemTime`（必须走 Clock port）、deny `println!`；workspace deny `unwrap` in 边界路径——把 harness 卡编译期化 |
| 验证 | CI 多 job 闸门链 + 修改必附测试 | 全部 gate 靠手跑/会话纪律 | **②** `.github/workflows/ci.yml`：双端 matrix + cargo tree gate + grep gate + test + （后续 public-api）——本文件 §三结构闸门全部自动化 |
| 指令 | **嵌套 CLAUDE/AGENTS.md ×11**（就近继承）+ settings.json hooks 自愈 | 仅 root CLAUDE.md + 1 个 rules 文件 | **③** 每 crate 一份薄 CLAUDE.md（core：零依赖铁律+不变量；im：剧本测试 pattern+ACL 约定；driver：worker/背压契约+压测命令）；PostToolUse hook 自动 rustfmt |
| 流程 | **.claude/skills 7 个 + commands 6 个**（pattern 即代码） | docs/harness 7 卡（雏形同构） | **④** harness 卡毕业为 `.claude/skills/`（写新模块 / 写回放测试 / 跑承压验收 三个 skill 起步） |
| 锁定 | rust-toolchain.toml + rustfmt.toml pinning | 无 | **⑤** 两个文件各 5 行，一次性 |
| 参照 | **Zig-as-reference**（.zig sibling 是行为真源） | 已在实践（现网 cses-client 逐文件核验）但未成文 | **⑥** 写进 root CLAUDE.md 一句：「行为争议时 `cses-client/src-tauri/features/im` 对应 handler 是 source of truth」 |
| 验证 | bench 42 域 + 无数字性能主张自动拒 | 压测三件套已入库但非常态 | **⑦** criterion 化 +「性能 PR 必附 before/after」写入规则 |

**已对齐不用补的**：结构化 commit + 模块化 tag（≈ Bun 的 PR 纪律）、对抗 review 流程（≈ Landing PR checklist 的 agent 化）、回放测试基建（≈ harness.ts 思想）、待办清单销账制（≈ issue 流程的轻量版）。

**Bun 给 5-crate 小项目的三条核心移植**（Explore 结论）：分层指令而非集中、clippy deny 当 API 契约、skills/commands 当 AI 课程——对应上表 ③①④。
