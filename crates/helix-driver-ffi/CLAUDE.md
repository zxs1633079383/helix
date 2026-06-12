# helix-driver-ffi 局部指令（就近优先，全局见根 CLAUDE.md）

## FFI 铁律（HX-C006）

1. 解引用裸指针的导出函数必须 `pub unsafe extern "C"` + `# Safety` doc（clippy 强制）
2. 每个 unsafe 块必配 `// SAFETY:` 注释（undocumented_unsafe_blocks = deny）
3. opaque ptr 单 owner：`Box::into_raw`/`from_raw` 各一次 + null-check
4. panic 策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 逐导出 catch_unwind——禁混说
5. 同步 SQLite C 调用必包 `spawn_blocking`（MAJ-4，落地前 todo 不许偷偷删）
6. C 侧 buffer 立即拷贝不持有（ACL-3）；读查询走独立同步 `helix_query`，不混 SendPort 事件流

## 改动验收

```bash
cargo check -p helix-driver-ffi && cargo clippy -p helix-driver-ffi
```
