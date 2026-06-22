## 04 — cses-client 接入方案（Phase P7，本轮只设计，零代码改动）

> **强调**：本轮 cses-client **不动任何代码**，仅产出接入设计。所有前面 Phase 都是 helix 侧实现，本文档是「helix 替换现网 IM 核心」的落地剧本。

### 1. 替换方式（二进制替换 + IPC 兼容层）

1. **二进制替换**：helix 编出 `helix-driver-native` 作为 lib；cses-client `src-tauri/Cargo.toml` 依赖 helix；现网 `src-tauri/src/features/im/` 整目录退役（保留为 `features/im_legacy/` 备份）。
2. **IPC 兼容层**：helix-driver-native 注册的 `#[tauri::command]` 命令名 + `im:__bus__` emit 与现网 byte-exact → `framework/boot.rs` 的 `generate_handler!` 列表只换 import 来源，**渲染层命令名不变**（表 A）；service 层出站/写命令改语义 invoke（表 B，是前端 service **重写**非零改，rv-tauri A2 诚实区分）。

### 2. 灰度开关

- env / `tauri.conf`：`IM_ENGINE=helix|legacy`；`boot.rs` 启动时按开关注册 helix 命令集或 legacy 命令集（**同名命令二选一注册**）。
- 灰度粒度：内部 dogfood（白名单 userId）→ 10% → 全量（开放决策 §6 待拍板具体下发机制）。

### 3. 前端纯渲染改造点（设计清单，不实现）

路径真源 `src/pages/message-v3`（非 `src/app/pages`，A3）：

| 文件 | 改造 |
|---|---|
| `messageSend.service.ts` | 删 buildMessageObject/15s timer/viewers 过滤/processMessageFiles 编排 → invoke `im_send_message`（args=UI 字段）；文件上传走 driver 直通拿 fileId 再 invoke |
| ~60 HTTP 直发点 | → 语义 invoke 命令（非泛型 im_outbound, C1）；鉴权/401/断网横切由 driver P1.5 层接管 |
| `store:* updateItem/setItem` 写点(43 处) | **全删**（写权收归 Rust 避免双写 PK 冲突, B2）→ 订阅 `im:channel:update`/`im:post:received` 拿完整 payload(遵 projection-schema) 直 setState |
| `im-sync.service.ts` | 仅保留 `onRustTooLong`（或改 Rust 拉首屏后 `im:post:increment` 投喂）→ 整 service 可删 |
| `tauri.service.ts` imDataSource 拦截垫片 | 删 query/queryCache/reset 3 个 early-return 拦截块(A4) → 真投影命令 `im_query_messages_by_channel`(复用 `store_get_messages` 语义 + 分页 beforeId/afterId/beforeTimestamp/afterTimestamp) |
| `networkInterceptor` IM 通道 | 退役（鉴权头/401/断网迁 driver, P1.5）；非 IM 通道保留 |

### 4. 回滚策略

- 灰度开关 `IM_ENGINE=legacy` 一键回滚（同名命令切回 legacy 注册）；但注意：service 层已改语义 invoke，回滚需同时 revert 前端 service（**回滚边界 = 前端 service 与 boot.rs 注册成对**，非纯后端开关）→ 建议灰度期前端 service 保留 legacy 分支（feature flag 双写）。
- 数据兼容：helix 与 legacy 同写 SQLite `message`(PK=temporaryId)/`channel`/`channel_event_cursor` 表，schema 对齐（helix 已移除 segment_id 死列，ADR-013；与 legacy IF NOT EXISTS 共存不冲突）→ 回滚不丢数据。

### 5. 联调验收

- `/im-dual-endpoint-log-debug` 双端日志(mattermost + cses-client)；4 段链路(emit→前端接力→HTTP 出站→服务端处理)。
- e2e：257 channel 真 Go connect/increment/sync（验 SyncScheduler 限流不风暴, B4）；send 6/8 echo 对账（验 action=="post" 锚, B3）；too_long 清内存重拉首屏（不清表）；切会话/上拉历史；401/断网横切端到端（验 P1.5）。
- gate：前端渲染零业务逻辑回归（unread/readBits/加急/回复全由 Rust 投喂，验 C2 投影 schema 字段齐）。

### 5.5 P6 落地后 IPC 面 finalize（按 P0-P6 真实代码勘误，2026-06-22）

> P6 接缝落地后，按 cses-client `framework/boot.rs` **真实注册的命令清单** + helix core 原语**真实能力**收口 IPC 面（rv-tauri 草案的若干 claim 与 ground truth 不符，此处勘误）。

**① `im_get_state` 不存在（rv-tauri B1「漏项」claim 勘误）**：核对 `boot.rs` generate_handler! 全量列表——**无** `im_get_state` 命令。rv-tauri 01 文档表 B「`im_get_state`/`im_delete_all_dialogs` 补入兼容表（boot.rs 已注册）」对 `im_get_state` 部分**与现网不符**。helix **不实现** `im_get_state`（凭空补 = 引入前端永不调用的死命令，违 YAGNI）。`im_delete_all_dialogs` 确实注册，保留。

**② 投影命令 `im_query_messages_by_channel` 经 `ScanSpec` additive 扩展落地**：现网是 `SELECT * FROM message WHERE channel_id=? ORDER BY create_at DESC, temporary_id DESC LIMIT ?`。core 原 `ScanSpec` 只有 `{table, limit}`（全表无过滤）/ `GetSpec` 单行——**均表达不了 per-channel 多行有序读**。P6 给 `ScanSpec` 加 **additive 可选字段** `filter: Option<(&'static str, SqlValue)>` + `order_desc_col: Option<&'static str>`（None = 旧全表语义零行为改变，HX-C004）。helix 走 `Scan(message, filter=channel_id, order_desc=create_at, limit)` → emit `im:messages:query_result` 投影。**保真偏差**：现网二级排序 `, temporary_id DESC`（同毫秒簇确定化）未携带（ScanSpec 当前单序列）——dump 用途下非语义关键；若未来需要，`order_desc_col` 升 `Vec<&'static str>` 即可（再 additive 扩）。

**③ `im_delete_all_dialogs` 物理 truncate = 核原语缺口（P7 blocker）**：现网 `DELETE FROM channel` + `DELETE FROM channel_member`（无条件全表删）。core `StorageOp` 无 truncate 原语（`BatchDelete` 需 `scope_col=? AND key_col IN (...)`，表达不了删全表）。P6 实现 = **清 core 内存 channel gate + emit `im:channels:loaded` 刷新**（命令名被识别，前端不报 command not found）；**物理 `DELETE FROM` 留待补 `StorageOp::Truncate`/`DeleteAll` 原语**——不为单个测试命令现造原语（YAGNI/HX-C001），列为 P7 接入前可选项。M2 共享既有 tenant.db 时 legacy 仍可物理删，helix 接管后该测试命令降级为内存清+刷新（功能影响仅限「测试用清库」按钮）。

**④ `createPosts` PascalCase 仍挂 blocker（D3 待真抓包）**：与 02 文档一致，未纳入 P6。

### 6. 接入前置 checklist（设计期确认项）

- [ ] 开放决策全部拍板（createUser/presence/createPosts casing/灰度机制/覆盖率工具）
- [ ] projection-schema.md 与 cses-client `DialogItem` interface 逐字段对齐（C2）
- [ ] 真 Go fixture 抓全（所有 casing 陷阱命令，C5）
- [ ] P1-P6 全 gate 绿 + 全仓 `cargo test -p helix-im` + host-cli e2e 通过
- [ ] 灰度 feature flag 前端双写分支就绪（回滚边界）
