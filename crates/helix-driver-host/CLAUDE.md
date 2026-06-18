# helix-driver-host 局部指令（就近优先，全局见根 CLAUDE.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 是什么

PC Tauri（native）与 Flutter FFI **共享**的 host driver 存储层（ADR-007）。SQLite schema/migration、rusqlite `Storage` 实现、`StorageOp → PortOutcome` 兑现、`rows_to_reply_bytes`/`rows_from_reply_bytes` 编解码——**统一维护在此**，`helix-driver-native` 与 `helix-driver-ffi` 只保留平台壳，不重复实现存储。

## 本 crate 不变量（破坏 = 双端存储漂移）

1. **单一真源**：messages SQL、monotonic 表、StorageOp 语义、rows 编解码 = 此 crate 唯一定义点；native/ffi 复用，禁各写一份（否则 PC 与手机端数据语义分叉）。
2. **rows↔bytes JSON 约定**：与 `helix-driver-native/engine_loop.rs` 头注对齐——Text→string / Integer→number / Real→number / Null→null；helix-im 反序列化端按此还原。
3. **热路径 O(1)**：batch_upsert 整批一事务（写放大 ~1.0x）；单 INSERT / UPDATE WHERE PK；禁 read-modify-write（HX-C005）。
4. **migration 不可逆**：只增不改历史 migration；schema 演进加新 migration，不回改旧的。
5. **driver 层 ambient 合法**：rusqlite 同步 C 调用、SystemTime 等在此/上层 allow；FFI 侧同步调用须 `spawn_blocking` 包裹（HX-C006/ADR-005，不阻塞 tokio worker）。

## 改动验收

```bash
cargo test -p helix-driver-host
# 改 schema/StorageOp/rows codec 后，native + ffi 两端都要回归（共享语义）
cargo check -p helix-driver-native -p helix-driver-ffi
```
