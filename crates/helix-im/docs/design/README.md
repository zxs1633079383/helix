# helix-im 设计图（mermaid · 当前已具备 IM 业务能力）

> IM 是跑在 helix-core（runtime-agnostic 确定性内核）上的**第一个业务模块**。
> 本目录的 `.mmd` 沉淀 helix-im **当前已落地**的能力与设计——全部从真实 `src/` 反推核实（非愿景）。
> 图例：✅ 已具备能力 · ⚠️ 边界/降级/兜底路径 · ❌ 缺口/stub。
>
> **不变量提醒**：handle 同步纯函数（副作用只经 `out.push(Effect::…)`，零 I/O 零 await）；
> 只依赖 helix-core；外部输入零 unwrap；行为真源 = 现网 `cses-client/src-tauri/src/features/im/`。
> 渲染：VS Code Mermaid 插件 / `mmdc -i x.mmd -o x.svg` / GitHub 原生预览。

## 索引

| 图 | 类型 | 一句话 | 真源 src | 关联 HX-C 卡 |
|---|---|---|---|---|
| [`01-ws-inbound-pipeline.mmd`](./01-ws-inbound-pipeline.mmd) | flowchart | **WS 帧入站全链路**：bytes → `WsFrame::parse`（一趟）→ registry by action（linkme+OnceLock O(1)）→ 7 类 handler → channel gate → Effect 落库 + `im:*` 事件 | `ws/{frame,registry}.rs` · `ws/handlers/*` · `channel.rs` | C005（`&'static str` 零分配 action）· C008 |
| [`02-send-echo-reconcile.mmd`](./02-send-echo-reconcile.mmd) | sequence | **发消息 + echo 对账**：`im_send_message` → 乐观上屏 + temporary_id → Http(post) → 真 Go → WS 回声 → reconcile（temporary_id→server_id 覆写 + 撤 15s timer），echo/timer 双次序终态一致 | `module.rs::handle_send_message` · `pending_send.rs` · `ws/handlers/{post,posted}.rs` | C006（对账确定性）· C008（cursor 不双推）|
| [`03-sync-flow.mmd`](./03-sync-flow.mmd) | flowchart | **同步流**：在线增量 push（increment_channel×N → end）+ 离线 pull（sync/notify，cursor 锚 nextSeq，SyncResponse 4 态）+ E3 续拉 + B2 冷启动自愈 + too_long 兜底 | `module.rs::{on_start,handle_scan_reply,handle_sync_reply,maybe_continue_sync}` · `ws/handlers/increment_channel_end.rs` · `sync_session.rs` | C008（nextSeq 锚 · len≥500 续拉 · too_long）· C010（迭代前 sort 确定性）|
| [`04-gate-state-machine.mmd`](./04-gate-state-machine.mmd) | stateDiagram | **gate 三阶段状态机**：Synced/Gapped + peek→落库→commit（写成功才推 cursor）+ cursor 单调不回退 + buffer 上限(16384)/服务端 too_long → 清内存 + drop local messages + 重拉 | `channel.rs`（`Channel::{ingest,on_persist_ok,peek_up_to,commit_cursor,reset_for_too_long}`）| C005（单 INSERT O(1) ON CONFLICT，无 RMW）· C008 |

## 当前能力速览（图覆盖的已落地清单）

- **WS 入站 dispatch ✅**：`WsFrame::parse` 一趟解析 → registry（linkme `distributed_slice` + `OnceLock` 按 action O(1)）→ 7 类 handler：
  hello 握手 / post（真 Go 在线新消息）/ posted（echo 对账）/ increment_channel / increment_channel_end / legacy_event（无 action）/ ignored（status_change·typing·presence·multiple_channels_viewed）。未知 action → `UnsupportedWsAction`。
- **发送 + echo 对账 ✅**：`im_send_message` → optimistic 落库 + `im:post:sending` + 15s timer → WS `post`/`posted` 回声 → `reconcile`（命中在途 `PendingSend` → server_id 覆写 + send_status='sent' + 撤 timer），未命中静默 no-op，超时标 `unsend`（无自动补发）。
- **同步 ✅**：在线增量 push + 离线 pull（cursor 锚 `next_seq` 非 `max(seq)`，HX-C008）+ E3 续拉（`needs_continuation`/len≥500，双重终止保证）+ B2 冷启动自愈（local cursor < increment 水位 → 补 sync，once-only）+ 确定性 resync（HX-C010：HashMap 迭代前 `sort_unstable`）。
- **gate 三阶段 ✅**：peek（只读）→ Persist 落库 → PortReply Ok 后 commit 推 cursor；cursor 单调不回退；buffer 上限 16384 或服务端 `too_long` → 清内存 buffer + drop 该 channel 本地 message 派生行 + reset Dialog 派生字段 + `im:sync:too_long` 全量重拉。
- **corr_map + CorrelationContext ✅**：统一关联态单表（4 变体：`ScanCursors` / `ChannelPersist` / `OptimisticSend` / `SyncPull`），corr 全局单调唯一保证类别互斥路由。
- **emit 11 个 `im:*` 事件 ✅**（真源 `acl/to_effect.rs`，经 `im:__bus__` 单总线）：
  `connection:established` / `channels:loaded` / `channel:increment` · `read` · `update` / `post:received` · `sending` · `updated` · `deleted` · `read` / `sync:too_long`。

## 维护铁律

- **真源同步**：`.mmd` 描述的是 `src/` 实际行为，源码改动后图必须同步（漂移即修）。
- **`im:*` 事件名唯一源** = `src/acl/to_effect.rs` 生成点；图中引用必须与之一致。
- **能力标注诚实**：只把 `src/` 已落地的标 ✅；stub / `todo!()` / 缺口标 ⚠️/❌，不画愿景。
- 当前 UC/HTTP/WS/存储综合口径见同目录上级 [`../前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](../前端UC投影指令与数据清单-最新代码分析-2026-07-09.md)；本目录只画**机制流程**。
