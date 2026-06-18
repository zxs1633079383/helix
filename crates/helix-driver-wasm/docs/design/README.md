# helix-driver-wasm — 设计图索引（📦 归档 / 暂停维护）

> **状态：archived（ADR-006，2026-06-15 拍板）。** 当前产品路线只维护 PC Tauri（native）+ Flutter（ffi）两端，网页端暂停。本 crate 保留为历史实验代码，**不进 roadmap / CI 必跑矩阵 / 交付验收**；`Storage` 等 ports 实现为 `todo!()` 占位。
>
> 若未来重启网页端：先新增 ADR 重新激活，再补 storage/cache 策略（OPFS/IndexedDB）+ wasm CI（HX-C003 仅作历史护栏保留）。

| 图 | 一句话 |
|---|---|
| [archived-web-architecture.mmd](./archived-web-architecture.mmd) | 📦 计划（暂停）的浏览器端架构——OPFS/IndexedDB Worker 存储 + web-sys WebSocket + spawn_local（无 tokio），留存供未来重启参考 |

## 三栖坑（若重启必读）
- OPFS 仅 `DedicatedWorker` 内可初始化（主线程崩）；SAHPool 单连接读写串行（无读池无并发）
- 无 tokio：spawn 用 `wasm_bindgen_futures::spawn_local`，通道用 `futures-channel`
- 单线程 `!Send`：不给 future 加 Send bound（HX-C002 wasm 段）
- wasm 分支只在 `--target wasm32-unknown-unknown` 下类型检查（HX-C003，native CI 看不见）

## 关联
- 决策：[`docs/harness/decisions.md`](../../../../docs/harness/decisions.md) ADR-006
- 全景：[`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) M4「归档的 wasm/web」
