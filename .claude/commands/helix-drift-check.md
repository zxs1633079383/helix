# /helix-drift-check — 文档 ↔ 代码对账

按以下清单核对权威设计文档（`docs/过会方案/helix-core-im-架构设计.md`）与 `crates/` 的一致性，输出「声明 | 代码实测 | 对齐/漂移」表：

1. Tick / Effect 变体清单 vs `tick.rs` / `effect.rs` 的 enum
2. 5 类 Port · 7 trait vs `ports/` 下 `pub trait` 实数
3. Module trait 方法数 vs `module_host.rs`
4. Storage 方法清单（含 scan）vs `ports/storage.rs`
5. 执行协议不变量逐条 vs `engine.rs` 实现（corr consume / timer peek / 排他路由）
6. engine_loop 五不变量 vs `crates/helix-driver-native/src/engine_loop.rs` 模块 doc
7. 文档内全部「可现场验证命令」实跑（grep 零业务 / cargo tree 零运行时）
8. 待办清单已销账项是否真在代码里（抽查 E/B/C 系编号的落点 file:line）

发现漂移：文档错 → 修文档；代码错 → 报告并按待办流程登记。**修复必须双向同步**（代码 + 文档 + 待办清单三处一致）。
