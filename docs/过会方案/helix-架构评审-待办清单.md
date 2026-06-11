# Helix 架构评审 · 待办清单（TODO）

> 来源：rust-dev 纯架构评审（2026-06-10，对照 cses-client 现网 IM 代码 + roadmap 评估）。
> 用途：过会后进入实现阶段的**必办事项清单**，防止评审揪出的缺口被遗忘。
> 📚 关联：[架构设计](./helix-core-im-架构设计.md) · [评审纪要](./helix-技术评审纪要.md) · 索引见项目 [`CLAUDE.md`](../../CLAUDE.md)。

---

## A. 已补入设计文档（评审 3 大缺口 · DONE）

- [x] **peek/落库/commit 三阶段**（写成功才推 cursor，防丢消息）→ 已补 [架构设计 §3.4](./helix-core-im-架构设计.md)
- [x] **hello 后 proactive resync**（UC-3/4 主动补漏所有 channel）→ 已补 [架构设计 §3.5](./helix-core-im-架构设计.md)
- [x] **driver 按 channel 并发兑现 Persist**（防退化单线程写）→ 已补 [架构设计 §2.2](./helix-core-im-架构设计.md)

---

## B. 实现待办 · 正确性 / 完整性遗漏（MVP2/3 必办）

> 这些是现网踩坑沉淀的真实机制，设计文档未展开，实现 Rust-im 时**必须显式处理**，否则会踩坑。

- [ ] **B1 · gate_inflight 去重守卫**（中）
  现网 `ImState.gate_inflight: HashSet<channel_id>` 防同 channel 并发触发多个 HTTP sync。
  Rust-im 需等价守卫：`Timer(gate)` 到期 + 「事件数≥阈值续拉」可能对同 channel 并发进 sync。不破坏正确性（cursor 单调 + upsert 幂等），但浪费双 HTTP/双 dispatch。
  落点：Rust-im `ImState` 加 inflight 集合。

- [ ] **B2 · increment_end 冷启动自愈 backfill**（中）
  现网 `handle_increment_end` → `proactive_resync_all_channels` + `heal_incomplete_backfill_once`（一次性自愈标志）+ `increment_fetched`（best-effort 去重）。
  proactive resync 框架已进设计 §3.5，但**自愈一次性标志 + backfill 去重**的细节实现待补。
  落点：Rust-im 响应 `Tick::Inbound{increment_channel_end}`。

- [ ] **B3 · type=6 readBits 多设备单调覆盖规则**（低-中）
  现网：sync 响应 type=6（已读 echo）→ 找 messages map 对应 msg 的 `read_bits` → **单调覆盖**（新值 > 旧值才写）。多设备已读同步的完整机制。
  落点：Rust-im handle sync events 的 type=6 分支 + `monotonic_upsert` 语义复用。

- [ ] **B4 · event_seq 三级 fallback 解析**（低）
  现网 `extract_event_seq_from_frame` 三级：① 顶层 `data.event_seq` → ② per-post `props.channel_event_seq` → ③ 顶层 `channel_event_seq`（legacy）。
  **`post_update`（edit）seq 在 per-post props，`posts_update`（delete）seq 在顶层** —— 位置不同，解析会踩坑。
  落点：Rust-im WS 帧 parser（`acl` 入站翻译）。

- [ ] **B5 · phantom 事件处理**（低）
  无 `is_phantom` 字段，靠 `messages.get(msg_id) == None` 判断：**event_seq 推进但消息 drop**（用户无权限看）。
  落点：Rust-im apply 事件时，msg 不在 messages map 则只推 cursor 不落库不 emit。

- [ ] **B6 · UpsertSpec conflict_key 的 ON CONFLICT UPDATE 列语义明确**（低）
  现网 `DO UPDATE SET col = excluded.col` 全字段全量覆盖。需明确：触发 conflict 时是否保留某些本地字段（如 `send_status`）。
  落点：core `UpsertSpec` 文档 + Rust-im ACL-1 约定。

- [ ] **B7 · store:\* / imDataSource 隐式契约对齐**（中 · 2026-06-10 四专家 review 新增）
  message-v3 消息读取实际走 `store_get_messages`（`(create_at, temporary_id)` 复合游标分页）等 `store:*` 命令族 + `tauri.service.ts` 内 `imDataSource:query/queryCache/reset` 前端旁路直调。Helix Storage driver 必须逐字段对齐 SQL 语义与 camelCase 入参映射，否则 Angular 读路径悄然失效。
  落点：driver-native Storage 实现 + 迁移验收 checklist（见架构设计 §4.4.1 接缝④）。

---

## C. 实现待办 · 约束 / 文档澄清（低优先，写清即可）

- [x] **C1 · Module::handle 同步约束 + MaybeSend 定义明确**（**2026-06-10 完成**）
  已落架构设计 §2.4「执行协议不变量」：handle 必须同步的三条依据 + ADR-001 并发模型（MaybeSend cfg 切换、native 专属单线程 LocalSet）+ ADR-002 状态 owned 禁 `&'a`。

- [x] **C2 · Effect 单一 enum 二进制层诚实补充**（**2026-06-10 完成**）
  已落架构设计 §五「诚实声明」：行为级解耦非编译级裁剪，Effect 单一 enum、IM 表名常量编进 wasm binary，唯一未来口子是 wasm 包体积。

- [x] **C3 · Storage 通用 scan 读能力**（⚠️ 曾升级为 MVP 前置阻塞项 · **2026-06-11 完成**）
  已落：core 新增 `ScanSpec` + `StorageOp::Scan` + `Storage::scan`（Phase A `7d39a11`）；native 实现（读池 + LIMIT 硬上限，Phase C）；helix-im on_start 已用（Phase B）。wasm/ffi 的 scan 仍为 todo 占位（随各端 driver 落地）。

- [x] **C4 · HX-C006 / CLAUDE.md panic 策略措辞修正**（低 · 2026-06-10 review 新增 · **同日完成**）
  现行表述「catch_unwind + panic="abort"」两者互斥：crate 级 panic=abort 使 catch_unwind 静默失效。已改为二选一表述（生产 cdylib abort / dev unwind+catch_unwind），与架构设计 §4.4.3 对齐。
  已修：`docs/harness/HX-C006-*.md`（含 §7 勘误记录）+ 项目 `CLAUDE.md` §2 checklist + `.claude/rules/runtime-agnostic-core.md` §7 与坑词典。

---

## D. Roadmap 调整建议（评审：整体 +3 周更现实）

- [ ] **D1 · MVP1 拉到 10-12 天**（原 7 天偏紧）
  wasm OPFS 必须 Worker 内初始化是硬坑（2-3 天）+ FFI uniffi layout 验证 + CI 三栖维护。

- [ ] **D2 · MVP1 wasm Storage 先用 IndexedDB 兜底**
  OPFS 复杂度高（origin isolation / COOP-COEP / Worker）。先用 IndexedDB（主线程可用、API 简单）跑通 Tick-Effect 循环，OPFS 留 MVP2/3 升级——否则「三栖验收」实为 native+ffi 两端。

- [ ] **D3 · MVP2 拆两 phase**（原 8-14 天过紧）
  `handlers/post.rs` 现网 1300+ 行，含 spawn/await/Mutex，重写为纯同步 Tick→Effect 是深度重构。
  - **MVP2a（~14 天）**：基础消息收发（UC-1 + 简单 UC-2）
  - **MVP2b（~10 天）**：完整增量同步（UC-2/3/4 + peek-commit + inflight + too_long + proactive resync + 所有边缘）

- [ ] **D4 · MVP3 前置测试基础设施**
  mock port + 可回放测试用例必须在 MVP3 验证前建好（喂剧本化 Tick 断言 Effect），否则只能靠真实环境验证，很慢。

- [ ] **D5 · MVP4 工期微调**
  Flutter FFI 4→7 天（uniffi 对 `Vec<Effect>`/嵌套 enum 有已知问题，需实测）；WASM 3 天只是 Demo，离生产可用（OPFS origin isolation / binary 优化）有距离。

---

## 评审总结论

> 架构能支撑「一份逻辑三端一致 + 可回放可测 + 加第二模块零改内核」，核心设计正确、与现网对齐度高、无颠覆性错误。**可以过会**——3 大缺口已补入设计（A），B/C/D 为实现阶段待办。工期建议整体 +3 周。
