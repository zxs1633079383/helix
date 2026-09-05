# Helix Web browser glue

这里仅包含无 UI 的浏览器平台接线：

- 浏览器持有物理 `WebSocket`，只把不透明帧交给 `HelixWebClient`；
- bootstrap 必须分别提供 Go `apiBaseUrl` 与 Java `defaultApiBaseUrl`；两者允许不同 origin，
  adapter 原样传给 Rust，禁止从 Go URL 裁路径或拼域名推测 Java 地址；
- 双地址槽不等于浏览器媒体上传已实现：当前 `Effect::UploadFile` 明确返回 unsupported，
  runtime issue 只报告能力缺失且不包含本地路径；本轮 rich/file 真实上传验收仅覆盖 native/FFI；
- WebSocket 鉴权只用 `URLSearchParams` 写入 `cookieId`，并清除基础 URL 中可能覆盖它的旧
  `userId`；不设置自定义握手 header；
- Rust 继续持有 HTTP、query/local-first、sync、cursor、echo 和业务帧处理；
- `fixture/index.html` 是真实浏览器 harness 入口，不是产品页面。

生成 wasm-bindgen 的 `pkg/` 后，用静态服务器托管 crate 目录。CLI 版本必须与
`Cargo.lock` 中的 crate 版本一致（当前为 `0.2.126`）：

```bash
cargo build --target wasm32-unknown-unknown -p helix-driver-web
cargo install wasm-bindgen-cli --version 0.2.126 --locked
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
node --test crates/helix-driver-web/browser/message-lifecycle-recorder.test.mjs
```

MessageLifecycle 的 Web/WASM 只保留 `MessageLifecycleRecorder` seam，供 bootstrap/WDIO 显式写入
T1 与 T2/T3/T4/T5 的结构化状态。它是 metadata-only、容量有界的 deterministic recorder，不
创建 OTLP exporter、不读取 message/body/headers/SQL，也不把缺失 HTTP 或 WS 转成伪造 span；本
阶段的 WASM exporter 不实现。
