# 分类接龙工作区审查与优化

本轮基于 `reactor/MessageV3-helix @ caf20f3b` 的既有未提交工作区，实施 DDD 边界、竞态和冗余修复。范围是当前 Helix 源码；未修改 Go、客户端、版本、锁文件或已有候选包/历史联调制品。

## 文字接龙约束

以下五个文件与本轮开始前及 HEAD 逐字一致：

- `crates/helix-im/src/chain.rs`
- `crates/helix-im/src/module/chain.rs`
- `crates/helix-im/src/outbound/posts/chain.rs`
- `crates/helix-im/src/ws/handlers/post_chain.rs`
- `crates/helix-im/src/port_reply/message_v3_chain.rs`

TEXT 的九个旧命令/HTTP 路由、缺省 mode、数字版本约定继续由分类生命周期测试中的兼容用例验证。`UpsertSpec::new` 的版本守卫默认关闭；普通 TEXT 消息保留原有直接 Emit 分支。混合同步批次中的文字接龙事件原字节立即释放，分类读回失败不会吞掉它们，新单元测试对此作了断言。

## 修复与设计依据

| 问题 | 实现 | 定向证据 |
|---|---|---|
| 分类 HTTP 新版先落库，普通 Post 旧版随后覆盖 | 通用 Upsert 可选 `version_column`，只由 CATEGORY_CHAIN 启用；`message.category_revision` 使用固定 20 位十进制文本，SQLite/Web 均在冲突更新处比较 | SQLite 公开 Module seam：HTTP revision 10 后依次收到旧版 post、post_update、posts_update，以及乱序缓冲的 post_update，message 和实际 Emit 均保留 10 |
| 挡住旧写但仍发出预先计算的旧事件 | 分类提交成功后读 durable message，再输出对应 post 投影；保持事件序号/关联元数据。同步批次复用相同读回接缝 | 上述回归必须实际收到 post 事件，不能因静默不发而通过；混合文字接龙/分类读失败测试验证隔离 |
| 本人参与版本未变，权限却因关闭/时间/访问变化而过期 | 参与条目保留版本化持久事实；四个权限字段作为本次响应的观察值，在持久条目读回后合并，不再被同版本缓存覆盖，也不为每次查询额外持久化一份权限收据 | 同 participationRevision，开放→关闭权限返回新值 |
| Web 缺行 Get 返回空 bytes，而 Host 返回 `[]` | Get/ScopedGet 缺行统一编码为 JSON 空数组 | Web 空读回归 |
| CAS 竞争失败被误当成存储损坏，永久禁用 Web store | 使用类型化 `PortError::StorageConflict`，事务仍回滚，但不设置永久故障标志；真实存储错误继续 fail-closed | SQLite 原子回滚、Web 冲突后可读、Web 原有故障锁存测试 |
| fact 的旧路径占用 Scan limit，截断新版事实树 | 新增并回填 `version_key`，建立 `(version_key,path)` 索引；先按版本过滤再限制行数，保留稳定 `(snapshot_key,path)` 主键 | 两版各 3000 个不重合字段的公开生命周期回归；旧 tenant schema 迁移重复执行测试 |
| 阶段 magic number 与重复查询分类 | 存储 continuation 直接匹配 Pending enum；outbound 共用既有 is_read；分类编辑消费已有 patch，去掉整份 PostFields 和 patch 的重复复制 | 编译、生命周期和热路径审计 |

DDD 处理保持既有 Sans-IO/六边形依赖方向：分类版本语义、权限解释和投影属于 helix-im；core 只增加通用可选写守卫和冲突类型；Host/Web 实现原子存储规则。保留现有串行工作队列与显式 continuation 状态机，不新增插件框架或通用策略注册层。

同步路径已接入同一写守卫与分类读回；本轮实际定向交错测试覆盖上表三类 WS 入口及缓冲释放，不把这些本地证据扩称为任意跨端/跨进程竞态的完整证明。

## 验证

所有 Cargo 命令均使用本地缓存 `CARGO_HOME=/tmp/helix-category-cargo-home`、`--offline --locked -j2`。

| 验证 | 结果 |
|---|---|
| 修复前定向 RED | 权限缓存 1 失败；Web 空 Get/冲突恢复 2 失败；旧 fact 路径截断 1 失败；普通 Post 覆盖 revision 1 失败 |
| `cargo test -p helix-im --lib` | 371 passed |
| `cargo test -p helix-im --test category_chain_lifecycle` | 31 passed，含 TEXT 九路由兼容 |
| Host `category_chain_storage` / `upsert_guard` / `storage_conformance` | 6 / 3 / 21 passed |
| `cargo test -p helix-driver-host --lib` | 89 passed；本机临时 HTTP 端口测试在允许 loopback 的沙箱授权后通过 |
| `cargo test -p helix-driver-web --lib` | 40 passed |
| core 单元/集成/doc tests | 23 passed，1 既有 ignored |
| core + IM + Native + FFI + Web `cargo check` | PASS |
| `cargo build --target wasm32-unknown-unknown -p helix-driver-web` | PASS |
| core `cargo tree` runtime/platform 依赖扫描 | tokio/sqlx/reqwest/web-sys/tauri 零命中 |
| Native release `stress_engine` / `stress_storage` | 4 / 3 passed |
| 分类 post/facts/runtime/readback 热路径审计；`git diff --check` | PASS；readback 的 EventSink JSON 编码作带原因的 lint 豁免，它不写任何存储列 |

承压本轮实测：5000 行 batch_upsert 10.362 ms；1 万次无窗口持久化回灌 498.140 ms；有窗口回灌 479.984 ms。这里只记录本机验收数字，不宣称相对修复前的性能提升。

### 未覆盖和既有阻塞

`cargo test -p helix-im` 全量尝试被未改动的 `tests/f1_gap1_reconnect_read_channel_test.rs` 阻塞：`ImConfig` initializer 缺 `client_platform`。没有扩大修复该历史问题，不声称仓库全量绿色。

本轮未重跑真实 Go/Native 业务链、FFI 消费链或真实浏览器/UI 联调。已有历史报告和制品保持原样，不用其旧结果冒充本轮验证。

保留未提交工作区：本轮修复依赖工作区既有的分类实现，并与该任务尚未提交的实现/制品共存；未把原始工作整体提交或发布。无 push。

## 本机复核材料

- 原始 dirty 文件快照及 SHA：`/tmp/helix-category-review-baseline/manifest.json`
- 本轮相对于开局工作区的 Rust 增量：`/tmp/helix-review-optimization.patch`
- 逐文件前后 SHA：`/tmp/helix-review-optimization-manifest.json`
- 最终结果：`/tmp/helix-review-validation-status.log`、`/tmp/helix-review-final-focused.log`、`/tmp/helix-review-im-lib.log`、`/tmp/helix-review-web-final.log`
- 平台与承压：`/tmp/helix-review-check-final.log`、`/tmp/helix-review-wasm-final.log`、`/tmp/helix-review-stress.log`
- 全量阻塞：`/tmp/helix-review-im-all.log`

/tmp 材料仅是本机本轮日志；本报告和源码测试是仓内可保留的复核入口。
