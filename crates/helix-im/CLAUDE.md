# helix-im 局部指令（就近优先，全局见根 CLAUDE.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 文档索引（`docs/` · 按需 Read）

- **落地真相** → [`docs/sync-v2-能力与缺口状态.md`](docs/sync-v2-能力与缺口状态.md)：现在到哪了 + 缺什么 + 今日必办（§5 真 Go 数据路径端到端验证）。
- **🔴 outbound 请求体真源（真抓包）** → [`docs/csesapi-migration/真机curl真源.md`](docs/csesapi-migration/真机curl真源.md)：现网实抓 6 curl 逐字 body+头；改任何 outbound 命令体（send/makeTopic/create/revoke/close/member）前**必读**。真抓包 > 源码推断。
- **csesapi → cses-im-server 迁移全量图谱** → [`docs/csesapi-migration/full-map/00-README-index.md`](docs/csesapi-migration/full-map/00-README-index.md)：现网后端 HTTP(124)/WS(19 事件) + 前端 message-v3 用例/UI 渲染/src-tauri Tauri 交互全清单 + 三态缺口矩阵（[`30-capability-gap-matrix.md`](docs/csesapi-migration/full-map/30-capability-gap-matrix.md)）。接口可达性 + 蓝图见 [`csesapi-migration/01-接口可达性清单.md`](docs/csesapi-migration/01-接口可达性清单.md) · [`02-迁移蓝图.md`](docs/csesapi-migration/02-迁移蓝图.md)。
- 根 CLAUDE.md/AGENTS.md §4.5.1 已挂同一索引（每次对话自动加载）。

## 本 crate 不变量

1. **只依赖 helix-core**：Cargo.toml 永不出现 tokio/sqlx/reqwest（clippy deny 兜底）
2. **handle 同步纯函数**：副作用只经 `out.push(Effect::…)`；异步走 corr 回调
3. **行为真源**：与现网行为有争议时，`cses-client/src-tauri/src/features/im/` 对应 handler 为准
4. **外部输入零 unwrap**：WS 帧 / sync 响应一律 Result（unwrap_used=warn 盯着）
5. **gate 机制铁律**：peek→落库→commit 三阶段；cursor 单调不回退；too_long 清内存不清表

## 测试 pattern（新逻辑必须配回放测试）

- 用 `tests/common.rs` 的剧本工具：构造 ImModule → 喂 Tick 序列 → 断言 Effect 序列
- 乱序/边界场景进 `tests/proptest_gate.rs` 性质测试
- 回归测试命名锚定待办编号：`tests/` 内以 `// 回归锚: E{N}/B{N}` 头注释标记（Bun 对齐 3.3）
- scan 回报解析必须对齐 driver 的 rows→bytes JSON 约定（engine_loop.rs `rows_to_reply_bytes`）

## 改动验收

```bash
cargo test -p helix-im                      # 全量（45+）
cargo test --release -p helix-im --test stress_test -- --test-threads=1   # 承压（改热路径时）
```
