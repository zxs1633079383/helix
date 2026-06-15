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

- [x] **B1 · gate_inflight 去重守卫**（中 · **2026-06-11 随 E3 落地**：Channel.inflight_sync + 全部 sync 发出路径置/清 + Disconnected 清理）
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

---

## E. 并发承压修复（2026-06-11 四专家审查 + 压测揪出 · 详见 [并发承压评估](./helix-并发承压评估-2026-06-11.md)）

- [x] **E1 · P0 泵死锁**（**2026-06-11 修复**）：独立 unbounded 回灌通道 + per-table-set 有序 worker + 有界 job queue(1024) 背压传导 + graceful drain（join 全部 worker，「返回=全落盘」）+ shutdown oneshot。验收：10k 裸灌不死锁；drain 测试断言 500 条全落盘。经对抗 review 3 blocking 回炉。
- [x] **E2 · P0 batch_upsert 包事务**（**2026-06-11 修复**）：begin/commit 整批 + SQL 串复用。实测 369ms→~40-60ms / 5000 行（82K-122K 行/秒），写放大 9.6x→~1.0x。
- [x] **E3 · P0 needs_continuation 续拉**（**2026-06-11 修复**）：persist 提交后续拉、fromSeq 严格递增终止保证、空 events 防死循环；顺手落地 **B1 gate_inflight**（per-channel 在途 sync 守卫 + Disconnected 清理）。
- [x] **E4 · P0 scan_corr 回报路由**（**2026-06-11 修复**）：handle_port_reply 路由 0 消费 Scan 回报；review 拦截「扫虚构表」后对齐 driver 真实 `_helix_monotonic(scope_key,value)` schema；解析失败降级空集启动。
- [x] **E5 · P1 monotonic_upsert DDL 出热路径**（**2026-06-12 修复**）：建表挪到 `open()` 一次性，热路径仅剩单 INSERT。回归锚 `test_monotonic_table_created_at_open_e5`。
- [x] **E6 · P1 gate buffer 容量上限 + 客户端主动 too_long**（**2026-06-12 修复**）：`MAX_GATE_BUFFER=16384`，超限清 buffer + emit too_long + sync 重拉，cursor 不回退。回归锚 ×2。
- [x] **E7 · P1 broadcast lagged 可感知**（**2026-06-12 修复**）：感知点从 emit(send)侧 改到接收侧 `EventReceiver/RecvOutcome::Lagged` 显式上报（纠正承压评估原定位——send 侧感知不到 lagged）。回归锚 ×2。
- [x] **E10 已落地（2026-06-12 `d58dff1`）**：写池 `PRAGMA synchronous=NORMAL`。实测 cursor FULL 8.1K→NORMAL warm ~16K（**~2x，非数量级**）。⚠️「fsync 822 calls/s」本仓从未实测、已 **retract**（macOS fsync 弱）；E10 增益平台依赖，生产 Linux 待测。详见承压评估 2026-06-12 节。
- [ ] **🔴 多模块并发写 benchmark（对抗审揪出的真缺口）**：单写者池（max_connections=1）下，第二模块（presence/docs）并发 batch_upsert 竞争同一写连接的实测——「通用业务海量涌入」的真问号，零数据。+ E9 cursor 热路径 to_string 堆分配 + wasm 存储 todo!() 待补。
- [ ] **E8/E9 · P2**：timer O(N) 查找反向索引；热路径分配收口（HX-C005）。

---

## F. UC-1 联调前置缺口（2026-06-12 四专家 panel-review 揪出 · 详见 ROADMAP §一 D1-2）

> 用 ROADMAP + 承压评估当输入跑 `expert-panel-review`，对抗证伪后存活 3 条，全部实锤（grep 验证）。UC-1 happy-path 跨的 3 块代码当时**死/缺/脆**，smoke 排在它们之前 = 顺序倒置。**用户已拍板：先补 4 块再跑真 Go smoke**。

- [ ] **F1 · 🔴critical echo→reconcile 路由缺失**：`handle_echo`（`module.rs:679`）`#[allow(dead_code)]`、Inbound 分支从不调用 → 发消息后 PendingSend 永停 Sending、tmp_id→正式 id 不覆写、「已送达」不 emit → UC-1 echo 验收必然不绿。修：Inbound 分支识别 echo 帧路由到 `handle_echo`。
- [ ] **F2 · 🟠high parser 无 event_seq fallback + hello-ack 解析**（`parser.rs:29-56`）：真 Go 帧含 event_seq / hello-ack 无 seq → `ImError::Parse` 静默丢 → cursor 不推/握手不成。修：B4 三级 fallback + hello-ack 帧格式。
- [ ] **F3 · 🟠high 握手时序 + `im:connection:established` 全仓 0 命中**：`Tick::Connected`（TCP/WS 完成即触发，`transport.rs:103-156`）早于 Go 应用层 hello-ack → resync 先于 auth 可能 401/403；且 established 事件**模块/driver 都没 emit**，D1-2 验收条件本不满足。修：Connected→发 hello→收 hello-ack→置 Connected + emit established，联调前先与 Go 对齐 hello 字段集。
- [x] **B 侧 E5/E6/E7 修复经同一 panel 对抗审 = 干净**（0 条存活，prod-align 提 3 条全证伪），无需返工。
