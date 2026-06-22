# helix-driver-wasm 局部指令（就近优先，全局见根 CLAUDE.md）

> ⚠️ **本 crate 已归档（ADR-006，2026-06-15）**。当前产品化只维护 **PC Tauri（native）+ 手机端 Flutter（FFI）**；网页端暂停维护，本 crate **不进 roadmap / CI 必跑矩阵 / 交付验收**。
> 下方指令仅作历史参考。若未来重启网页端，**先新增 ADR 重新激活本 crate 与 HX-C003**，再按下文施工。

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 三栖坑速查（完整见 .claude/rules/runtime-agnostic-core.md）

1. 本 crate 只在 `--target wasm32-unknown-unknown` 下被类型检查（HX-C003）——native CI 看不见你的类型错误
2. OPFS 仅 DedicatedWorker 内可初始化；SAHPool 单连接读写串行（无读池无并发）
3. 无 tokio：spawn 用 wasm_bindgen_futures::spawn_local；通道用 futures-channel
4. 单线程 `!Send`：不要给 future 加 Send bound
5. ~~M3 路线（评审 D2 拍板）：先 IndexedDB 兜底跑通循环，OPFS 后置升级~~（已随 ADR-006 归档，重启前不施工）

## 改动验收

```bash
cargo check --target wasm32-unknown-unknown -p helix-driver-wasm
```
