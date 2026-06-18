# helix_query 设计文档（blocked-on-IM，仅设计无实装）

> 真源：`crates/helix-driver-ffi/src/api.rs`（stub 现状）·
> `crates/helix-driver-host/src/storage.rs`（HostStorage，查询基础设施）·
> `crates/helix-core/src/ports/storage.rs`（Storage trait）。
>
> **本文件只写设计，不含实装代码。** 实装 blocked 待 helix-im 暴露通用 query 路由（见 §5）。

---

## 1. 现状：honest stub

```rust
// crates/helix-driver-ffi/src/api.rs
#[no_mangle]
pub extern "C" fn helix_query(
    handle: *mut HelixHandle,
    query_name: *const c_char,
    payload: *const u8,
    payload_len: usize,
    out_buf: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // FFI_UNIMPLEMENTED：运行时基础设施已就位，缺 query_name → Spec 路由（见设计文档）
    tracing::warn!("helix_query not yet implemented");
    HELIX_ERR_UNIMPLEMENTED
}
```

`helix_query` 是 FFI 层的**同步读路径**（C++ 调用、阻塞等结果），目前为 honest stub。

---

## 2. 运行时基础设施已就位

`HelixHandle` 持有两个 query 所需组件（`api.rs` 当前代码）：

```rust
pub(crate) struct QueryDeps {
    pub handle: tokio::runtime::Handle,   // pump runtime Handle
    pub storage: Arc<HostStorage>,        // 共享 HostStorage（与 pump 同实例）
}

pub struct HelixHandle {
    // ...
    pub(crate) query: QueryDeps,
}
```

- **pump runtime Handle**：`current_thread` runtime 的 `Handle`，可在 FFI 同步线程上用
  `handle.block_on(async { ... })` 安全调度 async 任务（不在 pump 线程本身调，避免死锁）。
  **必须用 `spawn_blocking` 包 rusqlite 同步 C 调用**（HX-C006），再在 FFI 同步线程
  `block_on` 等 `spawn_blocking` 完成 → 不阻塞 pump 单线程。
- **共享 HostStorage**：同一 `Arc<HostStorage>` 实例被 pump 和 query 共享，读路径经
  ReadPool（WAL 快照并发，不影响 pump 写路径）。

---

## 3. 设计问题：query_name → Spec 映射属 IM 领域语义

**核心约束（违反 = Storage trait 泄漏业务 schema）**：

`Storage` trait（helix-core ports）是通用接口：
```rust
// ScanSpec / GetSpec 不含业务 schema，table 名是 &'static str
pub struct ScanSpec { pub table: &'static str, pub limit: Option<i64> }
pub struct GetSpec  { pub table: &'static str, pub key_col: &'static str, pub key_val: SqlValue }
```

`query_name` 是 IM 业务概念（如 `"channel_messages"` / `"channel_list"` / `"unread_count"`），
把 `query_name → ScanSpec/GetSpec` 的映射写进 FFI driver 会：

1. **破坏 runtime-agnostic core（HX-C001 方向）**：FFI driver 不应知道 IM schema（messages/channels 表名）。
2. **破坏单一真源**：同样的 `query_name → table` 映射会同时存在于 FFI driver 和 helix-im 两处，任一表改名即二者漂移。
3. **违反 Storage trait 设计意图（`.claude/rules/runtime-agnostic-core.md` §6）**：
   > Storage trait 不许泄漏业务 schema（如 ON CONFLICT(temporary_id)）—— 用通用 UpsertSpec/GetSpec，IM 专属语义留 helix-im。

---

## 4. 正解设计：IM 暴露通用 query 路由，driver 只透传

**设计方向（待 helix-im 实现，FFI 侧 infra 不变）**：

```
C++ caller
   │ helix_query(name="channel_messages", payload={"channel_id":1})
   ▼
FFI driver (api.rs)
   │ 不解析 name → Spec；只持有 (Handle, Arc<HostStorage>)
   │ 把 (name, payload_bytes) 路由给 IM 注册的 query handler
   ▼
helix-im::query::QueryRegistry（待实现）
   │ name="channel_messages" → Fn(payload) → StorageOp → HostStorage
   ▼
HostStorage.scan/get → 行结果
   │
   ▼
FFI driver 序列化结果 → C++ out_buf
```

**QueryRegistry 形态（helix-im 负责实现，不在 driver 层）**：

```rust
// crates/helix-im/src/query.rs（待实现）
// driver 持 Arc<dyn QueryRouter>，IM 在 register() 时注入
pub trait QueryRouter: Send + Sync + 'static {
    fn route(
        &self,
        name: &str,
        payload: &[u8],
        storage: &dyn Storage,
    ) -> impl Future<Output = Result<Vec<Row>, PortError>> + Send;
}
```

- `QueryRouter` 在 core ports 层定义（泛型化，不引 IM schema）。
- helix-im 实现 `QueryRouter`：注册 `"channel_messages"` → `parse payload → ScanSpec → storage.scan()`。
- FFI driver 在 `HelixHandle` 持 `Arc<dyn QueryRouter>`（由 helix-im `register()` 时注入）。
- `helix_query` 调 `router.route(name, payload, &storage)` → `spawn_blocking` + `block_on`（HX-C006）。

---

## 5. FFI 侧 helix_query 实装草案（待 QueryRouter 接口定稳后填充）

```rust
// 待填充（infra 已就位，等 QueryRouter trait + helix-im 实现）
#[no_mangle]
pub extern "C" fn helix_query(
    handle: *mut HelixHandle,
    query_name: *const c_char,
    payload: *const u8,
    payload_len: usize,
    out_buf: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // 1. null-check + ffi_str 守卫（HX-C006 零信任）
    // 2. payload to_vec（C 侧 buffer 立即拷贝，ACL-3）
    // 3. handle.query.handle.block_on(async {
    //        spawn_blocking({
    //            router.route(name, &payload, &storage)  // ← QueryRouter 调用点
    //        }).await
    //    })
    // 4. 序列化 Vec<Row> → JSON bytes → Box<[u8]> → into_raw → out_buf/out_len
    // 5. return HELIX_OK
    HELIX_ERR_UNIMPLEMENTED
}
```

**阻塞调用链安全性**（C++ 在 FFI 同步线程调 `helix_query` → Rust block_on）：

- `handle.query.handle.block_on(...)` 在 **C++ 的调用线程**上执行 async future，
  **不占用 pump 线程**（pump 是独立 `std::thread`，有自己的 current_thread runtime）。
- `spawn_blocking` 把 rusqlite 同步调用卸到 tokio blocking pool（HX-C006），
  不阻塞 pump 单线程。
- 禁止在 pump 线程（loop 内）调 `block_on`（会死锁），仅 FFI 同步调用线程可用。

---

## 6. 备选方案：command + Emit 回读（无需 helix_query 扩展）

对低频、非实时的读需求，可避开 `helix_query` 路径：

1. C++ 调 `helix_command("query_channels", payload)`。
2. IM handler 处理后 `Effect::Emit(DomainEvent::QueryResult{...})`。
3. `FfiEventSink::flush` 批回 C++ `on_domain_events_batch`，C++ 在事件流里拿结果。

**适用场景**：初始化时全量加载 / 拉一次性列表。
**不适用**：C++ 需要同步阻塞等结果（UI 渲染前必须有数据），此类走 `helix_query` 同步路径更合适。

---

## 7. 当前状态与解封条件

| 维度 | 状态 |
|---|---|
| FFI 运行时 infra（Handle + HostStorage）| ✅ 已就位 |
| C-ABI 导出函数骨架 | ✅ 已就位（stub return HELIX_ERR_UNIMPLEMENTED）|
| QueryRouter trait（helix-core ports）| ❌ 待定义 |
| helix-im QueryRegistry 实现 | ❌ 待实现（blocked-on-IM）|
| HelixHandle 持 Arc\<dyn QueryRouter\> | ❌ 待接线 |

**解封触发**：helix-im 暴露 QueryRouter trait + 注册 channel/message/unread 路由 →
FFI 侧 infra 已就位，接线 `helix_query` 主干（约 50 行，无架构风险）。
