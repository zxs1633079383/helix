# helix-core 局部指令（就近优先，全局见根 AGENTS.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 不变量（违反 = 三栖碎）

1. **零运行时依赖**：永不 import tokio/sqlx/reqwest/web-sys/tauri——clippy deny + `cargo tree` 闸门双保险（HX-C001）
2. **零业务概念**：grep channel/message/temporaryId = 0；新公开类型过 public-api 闸门（纪要拍板 #2）
3. **step() 严格同步**：零 await / 零 I/O / 零 spawn；ambient 时间被 clippy deny（必须经 Clock port）
4. **执行协议**：corr 单调不复用 · PersistFire 不回报 · accepts 排他 first-match · one-shot timer 正常触发并派发成功后由 ExecutionShell 回收；提前取消用 CancelTimer，周期行为以新 id re-arm · 同 step 内 Effect 间无兑现顺序保证（因果经 corr 串联，EFFECT-5）（完整表见架构设计 §2.4）
5. 状态 owned 禁 `&'a`（ADR-002）；默认 `!Send`，MaybeSend cfg 切换（ADR-001）

## 改动验收

```bash
cargo check -p cses-helix-core && cargo check --target wasm32-unknown-unknown -p cses-helix-core
cargo test -p cses-helix-core
cargo tree -p cses-helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'   # 必须空
```

改 port trait / Effect / Tick 变体 = 三端 driver + helix-im 全要适配，先列影响面再动手。
