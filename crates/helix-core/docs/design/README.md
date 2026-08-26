# helix-core 设计流程图索引（mermaid）

> 沉淀 **helix-core**（运行时无关·零运行时·确定性业务内核基座）**当前已具备的能力 + 设计**。
> 每张图均按真实 `src/` 代码绘制（非凭空编），文件名前缀编号即阅读顺序。
> 图例：`✅` 已具备 · `⚠️` driver 侧实现/部分接线 · `❌` 已归档（wasm，见 ADR-006）。
>
> **真源文件**：`src/{engine.rs, module_host.rs, tick.rs, effect.rs, platform.rs, error.rs, ports/*.rs}`
> 改 port trait / Effect / Tick 变体 = PC Tauri/native + Flutter FFI 交付 driver 与 helix-im 全要适配；
> wasm driver 仅归档跟随 trait shape（见本 crate `AGENTS.md` / ADR-006）。

## 图清单

| 文件 | 一句话 | 覆盖的真源 | 关联 HX-C 卡 / 不变量 |
|---|---|---|---|
| [`01-step-event-loop.mmd`](01-step-event-loop.mmd) | `ExecutionShell::step` 同步纯函数路由全景：6 Tick 进 → 三类路由（accepts 扫描 / corr_map O(1) / timer_map O(1)）→ `Module::handle` 吐 `Vec<Effect>` → 注册 corr/timer → 返回 `&[Effect]` | `engine.rs::step` / `register_effects_to_maps` · `module_host.rs::dispatch_*` | step() 零 await/IO/spawn · HX-C001(零运行时) · EFFECT-3 |
| [`02-effect-storageop-taxonomy.mmd`](02-effect-storageop-taxonomy.mmd) | 9 个 `Effect` 变体按「带 corr 必达 / Fire-and-forget / 其余无 corr」三类 + 7 个 `StorageOp` 子树 | `effect.rs::Effect` / `StorageOp` / 各 `*Spec` | HX-C005(热路径零分配 `&'static str`) · HX-C007(Effect 批处理) · EFFECT-5(无序契约) |
| [`03-module-lifecycle-routing.mmd`](03-module-lifecycle-routing.mmd) | `Module` 生命周期（register → start_one/on_start → 运行期定向路由 → on_stop 逆序）+ 多模块 corr 定向（EFFECT-3 只扫新增区间） | `module_host.rs::Module` / `ModuleHost` / `start_one` · `engine.rs::start` | 开闭原则(加模块只 register) · EFFECT-3(防 PortReply 投错家) |
| [`04-sans-io-boundary.mmd`](04-sans-io-boundary.mmd) | sans-IO 因果链时序：core 同步吐 Effect 意图 ──▶ driver async 兑现 ──▶ `PortReply` 经 corr 串联回灌 → 可回放确定性 | `engine.rs`(同步) + 6 ports(async 在 driver) + `effect.rs`(corr) | HX-C001(core 零运行时) · HX-C002(async port Send 边界) · 确定性/sans-IO |
| [`05-ports-registry.mmd`](05-ports-registry.mmd) | 6 ports（FrameSender/Storage/Clock/EventSink/IdSource+Random/HttpRequester）抽象 + PC/Flutter 实现去向 + `MaybeSend/MaybeSync` cfg 切 + 统一 `PortError` 对象安全 | `ports/*.rs` · `platform.rs` · `error.rs::PortError` | HX-C001(只定义 trait) · HX-C002(MaybeSend cfg) · 对象安全(无关联类型) |

## 速记不变量（与图配套）

- **零运行时**：`helix-core` 永不 import tokio/sqlx/reqwest/web-sys/tauri，只定义 trait；`cargo tree -p cses-helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 必须空（HX-C001）。
- **step() 严格同步**：零 await / 零 I/O / 零 spawn；`Module::handle` 纯函数，副作用一律 `out.push(Effect)`。
- **定向路由不误吞**：`Inbound`/`Command` 走 `accepts()` first-match；`PortReply` 走 `corr_map.consume` O(1)；`Timer` 走 `timer_map.peek` O(1)。
- **Effect 无序契约（EFFECT-5）**：同一 step 产出的 `Vec<Effect>` 之间无兑现顺序保证，因果必须经 `Correlation` 串联，不可依赖 `Vec` 下标。
- **确定性可回放**：时间/随机/ID 由 host 经 port 注入，相同 Tick 序列 + 相同模块初态 ⇒ 相同 Effect 序列。

## 渲染

mermaid `.mmd` 可在支持 mermaid 的 Markdown 预览 / mermaid.live / VS Code mermaid 插件直接渲染。
