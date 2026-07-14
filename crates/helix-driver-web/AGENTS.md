# helix-driver-web 局部指令（就近优先，全局见根 AGENTS.md）

> A4 Rust 平台壳已交付；A5 真浏览器/真 Go glue 与正式 CI gate 尚未交付。本 crate 不包含前端页面。

## 冻结边界

1. 本 crate 只兑现浏览器平台 I/O：Browser Fetch、page-session store、外部物理 WS bridge、timer 与批量事件发布。
2. 不解析 IM 帧，不复制 query/remote fallback、sync、sequence/cursor、echo 或业务 URL/body；这些只属于 `helix-im`。
3. session store 只在当前页面会话内有效，不跨刷新恢复。禁止在消息/coverage 未恢复时单独恢复一个超前 cursor。
4. UI 只调用 business invoke/query。`ws_connected`、`ws_frame`、`ws_disconnected` 与 platform actions 仅供 bootstrap glue 接线，不是业务 API。
5. `inventory` 的 WASM constructors 初始化属于 Web final artifact/bootstrap；不得下沉进 `helix-im`，也不得另建 Web 专用业务注册表。
6. 保持浏览器单线程/`!Send` 边界；不得为迁就 native 把 tokio、rusqlite、reqwest 或 `helix-driver-host` 引入本 crate。
7. 所有供 JS 轮询/drain 的输出队列必须显式有界；溢出要么 fail-closed，要么通过版本化 gap/drop 计数明确暴露，禁止静默丢失。
8. `ws_connected` 返回错误时，bootstrap glue 必须立即关闭刚建立但未被 bridge 接管的物理 socket；Rust bridge 会保留原连接状态，不替 JS 持有该失败连接。

## 改动验收

```bash
cargo build --target wasm32-unknown-unknown -p helix-driver-web
cargo test -p helix-driver-web
```

涉及业务 Tick/Effect 时，还必须保留 native/FFI/Web 同剧本 parity；真浏览器链路与 HX-C003 激活按 A5 推进。
