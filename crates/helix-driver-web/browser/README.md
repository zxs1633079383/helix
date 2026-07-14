# Helix Web browser glue

这里仅包含无 UI 的浏览器平台接线：

- 浏览器持有物理 `WebSocket`，只把不透明帧交给 `HelixWebClient`；
- WebSocket 鉴权只用 `URLSearchParams` 写入 `cookieId`，并清除基础 URL 中可能覆盖它的旧
  `userId`；不设置自定义握手 header；
- Rust 继续持有 HTTP、query/local-first、sync、cursor、echo 和业务帧处理；
- `fixture/index.html` 是真实浏览器 harness 入口，不是产品页面。

生成 wasm-bindgen 的 `pkg/` 后，用静态服务器托管 crate 目录。CLI 版本必须与
`Cargo.lock` 中的 crate 版本一致（当前为 `0.2.122`）：

```bash
cargo build --target wasm32-unknown-unknown -p helix-driver-web
cargo install wasm-bindgen-cli --version 0.2.122 --locked
wasm-bindgen \
  --target web \
  --out-dir crates/helix-driver-web/pkg \
  --out-name helix_driver_web \
  target/wasm32-unknown-unknown/debug/helix_driver_web.wasm
python3 -m http.server 4173 --directory crates/helix-driver-web
```

浏览器测试 harness 打开 `browser/fixture/index.html`，等待
`__HELIX_WEB_SMOKE_READY__`，再调用 `startHelixWebSmoke({ config })`。fixture 另暴露
`helixWebSmokeInvoke(name, bytes)`、`helixWebSmokeSnapshot()`、
`helixWebSmokeDrain()` 与 `stopHelixWebSmoke()`；历史数组有界，溢出计数随 snapshot
返回。

合同测试：

```bash
node --test crates/helix-driver-web/browser/helix-browser-adapter.test.mjs
```
