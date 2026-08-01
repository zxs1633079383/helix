# Phase 1 三端 Rust 验收

日期：2026-07-31

## 结果

- `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web`：exit 0。
- `cargo test -p helix-core --no-fail-fast`：exit 0；lib 16、frame sender 1、multi-module 1、replay 4、doctest 1 通过，1 个 doctest ignored。
- `cargo build --target wasm32-unknown-unknown -p helix-driver-web`：exit 0。

## 证据边界

上述结果证明 Native/FFI/Web 三个 Rust 构建面和 `helix-core` focused 测试面可用，不证明真实 PC/Mobile 宿主、双用户、外部 HTTP/WS 或 canonical Gate 已通过。
