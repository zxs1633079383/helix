//! WasmStorage — sqlite-wasm-rs + OPFS
//!
//! ## OPFS 初始化约束（MAJ-3）
//!
//! 必须在 **Web Worker** 内初始化（不能在主线程）：
//! - 主线程调用 `FileReaderSync` 抛 `InvalidStateError`（W3C 规范禁止）
//! - `SharedArrayBuffer`（OPFS sahpool VFS 依赖）需要 Cross-Origin Isolation headers
//!
//! `WasmStorage::new()` 在构造时检测上下文，若非 Worker 则返回 `Err` 让调用方决定降级。
//!
//! ## IndexedDB 备选
//!
//! 若 OPFS 不可用（Safari 旧版 / 非 Worker 上下文），可降级为 IndexedDB 适配器。
//! 替换此文件即可，port 契约不变。
//!
//! ## 编译要求
//!
//! RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals"

use helix_core::effect::{
    BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec, Row, ScanSpec, UpsertSpec,
};
use helix_core::PortError;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

pub struct WasmStorage {
    // TODO: db: sqlite_wasm_rs::Database,
}

impl WasmStorage {
    /// 构造 WasmStorage。
    ///
    /// MAJ-3: 检测 Worker 上下文，若在主线程调用则返回 Err。
    /// 调用方应在 Web Worker 内调用此函数，或在收到 Err 时降级到 IndexedDB。
    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Result<Self, PortError> {
        // 检测当前全局对象是否为 DedicatedWorkerGlobalScope
        // js_sys::global() 在主线程返回 Window，在 Worker 返回 DedicatedWorkerGlobalScope
        let global = js_sys::global();
        let is_worker = global
            .dyn_ref::<web_sys::DedicatedWorkerGlobalScope>()
            .is_some();

        if !is_worker {
            return Err(PortError::Storage(
                "WasmStorage (OPFS) must be initialized inside a Web Worker, \
                 not on the main thread. Consider using IndexedDB as a fallback, \
                 or move initialization to a dedicated worker."
                    .to_string(),
            ));
        }

        // TODO: 初始化 sqlite-wasm-rs OPFS database
        Ok(Self {})
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl helix_core::ports::Storage for WasmStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        let _ = spec;
        todo!("WasmStorage::batch_upsert")
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        let _ = spec;
        todo!("WasmStorage::batch_update")
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        let _ = spec;
        todo!("WasmStorage::monotonic_upsert")
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        let _ = spec;
        todo!("WasmStorage::guarded_bump")
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        let _ = spec;
        todo!("WasmStorage::get")
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        // SAHPool 单连接顺序执行（无读池）
        let _ = spec;
        todo!("WasmStorage::scan")
    }
}
