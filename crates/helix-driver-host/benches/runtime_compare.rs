// P3 runtime_compare bench — 已替换为诚实测试，见下方说明。
//
// ## 原设计的方法学问题（verify lens MAJOR-2，2026-06-18 修复）
//
// 原实现用 `b.iter(|| rt.block_on(单 op))` 测单 op **串行延迟**，不是并发吞吐。
// 两个 criterion group 独立测各自的单 op，没有同一负载下的对照，判据（<15%）只写
// 注释无 assert，criterion 也不提供跨 group 比较断言。这是 false-green bench：
// 永远绿，什么都证明不了。
//
// ## 为什么不需要 bench 来证明等同
//
// storage ops 全部走 `spawn_blocking` 卸到 tokio blocking pool。blocking pool 大小
// **不受** `current_thread` 单 worker 线程限制（tokio 设计规范）：
//   - `new_current_thread().enable_all()` = 单 event-loop worker + 独立 blocking pool
//   - `new_multi_thread().enable_all()`   = N event-loop workers  + 独立 blocking pool
//
// 两种形态的 blocking pool 使用同一实现，存储层吞吐几乎完全由 rusqlite I/O 决定，
// runtime 线程模型对纯 I/O 路径的影响可忽略。这是**构造上的等同**，无需实测。
//
// 线程模型真正的差异体现在 **engine 泵层**（tokio worker 数量影响 spawn_local 并发
// 与 future 调度公平性），由 helix-driver-native/tests/stress_engine.rs 的 P2 测试
// 覆盖（在 multi_thread runtime 下跑真并发 engine flood + 读写混合）。
//
// ## 结论
//
// P3 bench 已从 Cargo.toml [[bench]] 注销（保留此文件只为记录决策，不被 harness 运行）。
// 若将来需要实测存储吞吐对照，应写成 `#[tokio::test]`（放 host/tests），
// 使用 `join_all(N 并发 spawn)` 测真并发 throughput，而非 `b.iter(block_on(单 op))`。
